//! Parsing for S3 `ListObjectsV2` XML responses against the `noaa-mrms-pds`
//! bucket.
//!
//! Adapted from `provider-gefs`'s `xml.rs` (itself adapted from
//! `radar-cache`'s -- same job, same bucket API, same `quick-xml` version
//! already pinned in this workspace) rather than duplicated as brand-new
//! parsing logic: `quick-xml`'s bounds-checked pull parser is used the same
//! way, and this response body is exactly as untrusted, remote-sourced data
//! here as it is there (GLOBAL_CONTRACT:
//! "remote data is unreliable and untrusted" / "no uncontrolled panics on
//! malformed input") -- every malformed/truncated shape returns a
//! structured [`MrmsError::MalformedListing`], never a panic.
//!
//! Only the fields this crate's discovery needs are extracted: each
//! `<Contents><Key>` value, and the pagination fields `<IsTruncated>` /
//! `<NextContinuationToken>`.

use crate::error::MrmsError;
use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ListObjectsPage {
    pub keys: Vec<String>,
    pub is_truncated: bool,
    pub next_continuation_token: Option<String>,
}

pub(crate) fn parse_list_objects_v2(url: &str, xml: &[u8]) -> Result<ListObjectsPage, MrmsError> {
    let malformed = |message: String| MrmsError::MalformedListing {
        url: url.to_string(),
        message,
    };

    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);

    let mut page = ListObjectsPage::default();
    let mut in_contents = false;
    let mut current_tag = String::new();
    let mut depth: i64 = 0;

    loop {
        let event = reader.read_event().map_err(|e| malformed(e.to_string()))?;

        match event {
            Event::Eof => break,
            Event::Start(start) => {
                depth += 1;
                let name = start.name().as_ref().to_string();
                if name == "Contents" {
                    in_contents = true;
                }
                current_tag = name;
            }
            Event::Empty(empty) => {
                current_tag = empty.name().as_ref().to_string();
            }
            Event::End(end) => {
                depth -= 1;
                if depth < 0 {
                    return Err(malformed(format!(
                        "unmatched closing tag </{}> with no corresponding open tag",
                        end.name().as_ref()
                    )));
                }
                if end.name().as_ref() == "Contents" {
                    in_contents = false;
                }
                current_tag.clear();
            }
            Event::Text(text) => {
                let raw: &str = text.as_ref();
                let decoded =
                    quick_xml::escape::unescape(raw).map_err(|e| malformed(e.to_string()))?;
                if depth == 0 && !decoded.trim().is_empty() {
                    return Err(malformed(format!(
                        "unexpected top-level text content: {decoded:?}"
                    )));
                }
                match current_tag.as_str() {
                    "Key" if in_contents => page.keys.push(decoded.into_owned()),
                    "IsTruncated" => page.is_truncated = decoded.eq_ignore_ascii_case("true"),
                    "NextContinuationToken" => {
                        page.next_continuation_token = Some(decoded.into_owned())
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    if depth != 0 {
        return Err(malformed(format!(
            "document ended with {depth} element(s) still open (truncated or malformed input)"
        )));
    }

    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_real_mrms_listing_shape() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>noaa-mrms-pds</Name>
  <Prefix>CONUS/PrecipRate_00.00/20260913/</Prefix>
  <KeyCount>2</KeyCount>
  <MaxKeys>1000</MaxKeys>
  <IsTruncated>false</IsTruncated>
  <Contents>
    <Key>CONUS/PrecipRate_00.00/20260913/MRMS_PrecipRate_00.00_20260913-050000.grib2.gz</Key>
    <Size>564445</Size>
  </Contents>
  <Contents>
    <Key>CONUS/PrecipRate_00.00/20260913/MRMS_PrecipRate_00.00_20260913-050200.grib2.gz</Key>
    <Size>563000</Size>
  </Contents>
</ListBucketResult>"#;

        let page = parse_list_objects_v2("u", xml).unwrap();
        assert_eq!(
            page.keys,
            vec![
                "CONUS/PrecipRate_00.00/20260913/MRMS_PrecipRate_00.00_20260913-050000.grib2.gz"
                    .to_string(),
                "CONUS/PrecipRate_00.00/20260913/MRMS_PrecipRate_00.00_20260913-050200.grib2.gz"
                    .to_string(),
            ]
        );
        assert!(!page.is_truncated);
        assert_eq!(page.next_continuation_token, None);
    }

    #[test]
    fn parses_a_paginated_truncated_response() {
        let xml = br#"<ListBucketResult>
  <IsTruncated>true</IsTruncated>
  <NextContinuationToken>abc123==</NextContinuationToken>
  <Contents><Key>a</Key></Contents>
</ListBucketResult>"#;
        let page = parse_list_objects_v2("u", xml).unwrap();
        assert_eq!(page.keys, vec!["a".to_string()]);
        assert!(page.is_truncated);
        assert_eq!(page.next_continuation_token.as_deref(), Some("abc123=="));
    }

    #[test]
    fn rejects_truncated_xml_without_panicking() {
        let xml = br#"<ListBucketResult><Contents><Key>a</Key><Size>683"#;
        assert!(parse_list_objects_v2("u", xml).is_err());
    }

    #[test]
    fn rejects_malformed_xml_without_panicking() {
        assert!(parse_list_objects_v2("u", b"not xml <<<>>>").is_err());
    }

    #[test]
    fn rejects_mismatched_tags_without_panicking() {
        let xml = br#"<ListBucketResult><Contents><Key>foo</Contents></Key></ListBucketResult>"#;
        assert!(parse_list_objects_v2("u", xml).is_err());
    }

    #[test]
    fn rejects_invalid_utf8_without_panicking() {
        let xml: &[u8] =
            b"<ListBucketResult><Contents><Key>\xff\xfe</Key></Contents></ListBucketResult>";
        assert!(parse_list_objects_v2("u", xml).is_err());
    }
}
