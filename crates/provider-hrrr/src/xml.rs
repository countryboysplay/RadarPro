//! Parsing for S3 `ListObjectsV2` XML responses against the
//! `noaa-hrrr-bdp-pds` bucket. Adapted from `provider-gefs::xml` (same job,
//! same bucket API, same `quick-xml` version already pinned in this
//! workspace) rather than duplicated as brand-new parsing logic.

use crate::error::HrrrError;
use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ListObjectsPage {
    pub keys: Vec<String>,
    pub is_truncated: bool,
    pub next_continuation_token: Option<String>,
}

pub(crate) fn parse_list_objects_v2(url: &str, xml: &[u8]) -> Result<ListObjectsPage, HrrrError> {
    let malformed = |message: String| HrrrError::MalformedListing {
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
    fn parses_a_real_hrrr_listing_shape() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>noaa-hrrr-bdp-pds</Name>
  <Prefix>hrrr.20260912/conus/hrrr.t12z.wrfsfcf00.grib2</Prefix>
  <KeyCount>2</KeyCount>
  <MaxKeys>1000</MaxKeys>
  <IsTruncated>false</IsTruncated>
  <Contents>
    <Key>hrrr.20260912/conus/hrrr.t12z.wrfsfcf00.grib2</Key>
    <Size>690694451</Size>
  </Contents>
  <Contents>
    <Key>hrrr.20260912/conus/hrrr.t12z.wrfsfcf00.grib2.idx</Key>
    <Size>60649</Size>
  </Contents>
</ListBucketResult>"#;

        let page = parse_list_objects_v2("u", xml).unwrap();
        assert_eq!(
            page.keys,
            vec![
                "hrrr.20260912/conus/hrrr.t12z.wrfsfcf00.grib2".to_string(),
                "hrrr.20260912/conus/hrrr.t12z.wrfsfcf00.grib2.idx".to_string(),
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
