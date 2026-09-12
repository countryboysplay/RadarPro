//! Parsing for S3 `ListObjectsV2` XML responses.
//!
//! The response body is untrusted, remote-sourced data (GLOBAL_CONTRACT:
//! "remote data is unreliable and untrusted" / "no uncontrolled panics on
//! malformed input"): [`parse_list_objects_v2`] uses `quick-xml`'s
//! bounds-checked pull parser and returns a structured
//! [`crate::discovery::DiscoveryError::MalformedXml`] on anything
//! malformed or truncated, never panicking.
//!
//! Only the handful of fields discovery needs are extracted: each
//! `<Contents><Key>` value, and the pagination fields `<IsTruncated>` /
//! `<NextContinuationToken>`. Everything else in the response
//! (`<ETag>`, `<Size>`, `<StorageClass>`, ...) is ignored.

use crate::discovery::DiscoveryError;
use quick_xml::events::Event;
use quick_xml::Reader;

/// One page of a (possibly paginated) `ListObjectsV2` response.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ListObjectsPage {
    /// Every `<Contents><Key>` value in this page, in the order the server
    /// returned them.
    pub keys: Vec<String>,
    pub is_truncated: bool,
    pub next_continuation_token: Option<String>,
}

pub(crate) fn parse_list_objects_v2(xml: &[u8]) -> Result<ListObjectsPage, DiscoveryError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);

    let mut page = ListObjectsPage::default();
    // Tracks whether we are currently nested inside a <Contents> element,
    // since <Key> also appears (with different meaning) nowhere else in
    // this response shape, but tracking this explicitly is cheap and keeps
    // the parser correct even if that assumption is ever wrong.
    let mut in_contents = false;
    // The innermost currently-open tag's local name, so a following Text
    // event knows which field it belongs to. `ListObjectsV2` responses are
    // not deep enough to need a full tag stack for the fields we read.
    let mut current_tag = String::new();
    // `quick-xml`'s pull parser is deliberately lenient about a document
    // that simply stops partway through (e.g. a connection dropped
    // mid-response): reaching EOF with text still "in progress" or a tag
    // still open is not by itself a parser-level `Err` (verified
    // empirically: a body truncated mid-`<Size>683` parses as a `Start`
    // event with no matching `End`, then `Eof`, with no error). Since a
    // genuinely truncated/malformed S3 response must still be rejected
    // (never silently treated as "just an empty/small result"), track
    // open-element depth ourselves and require it to be back to zero by
    // EOF; a negative depth (an unmatched closing tag) is rejected the
    // moment it happens rather than waiting for EOF.
    let mut depth: i64 = 0;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| DiscoveryError::MalformedXml(e.to_string()))?;

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
                // A self-closing tag has no following Text event; nothing
                // to record for the fields we care about, but keep the
                // bookkeeping consistent in case a future field cares.
                current_tag = empty.name().as_ref().to_string();
            }
            Event::End(end) => {
                depth -= 1;
                if depth < 0 {
                    return Err(DiscoveryError::MalformedXml(format!(
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
                let decoded = quick_xml::escape::unescape(raw)
                    .map_err(|e| DiscoveryError::MalformedXml(e.to_string()))?;
                if depth == 0 && !decoded.trim().is_empty() {
                    // Non-whitespace text outside of any element: not a
                    // shape `ListObjectsV2` ever produces, so this is not
                    // XML we recognize at all.
                    return Err(DiscoveryError::MalformedXml(format!(
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
        return Err(DiscoveryError::MalformedXml(format!(
            "document ended with {depth} element(s) still open (truncated or malformed input)"
        )));
    }

    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_normal_response() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>unidata-nexrad-level2</Name>
  <Prefix>2026/09/12/KTLX/</Prefix>
  <KeyCount>2</KeyCount>
  <MaxKeys>1000</MaxKeys>
  <IsTruncated>false</IsTruncated>
  <Contents>
    <Key>2026/09/12/KTLX/KTLX20260912_000110_V06</Key>
    <LastModified>2026-09-12T00:04:32.000Z</LastModified>
    <ETag>&quot;6faecebf15f26fe4a84f2baae09ba227&quot;</ETag>
    <Size>6834037</Size>
    <StorageClass>STANDARD</StorageClass>
  </Contents>
  <Contents>
    <Key>2026/09/12/KTLX/KTLX20260912_000440_V06</Key>
    <LastModified>2026-09-12T00:08:02.000Z</LastModified>
    <ETag>&quot;a53c39ce13228ae90fb32988dc08b79d&quot;</ETag>
    <Size>6740332</Size>
    <StorageClass>STANDARD</StorageClass>
  </Contents>
</ListBucketResult>"#;

        let page = parse_list_objects_v2(xml).unwrap();
        assert_eq!(
            page.keys,
            vec![
                "2026/09/12/KTLX/KTLX20260912_000110_V06".to_string(),
                "2026/09/12/KTLX/KTLX20260912_000440_V06".to_string(),
            ]
        );
        assert!(!page.is_truncated);
        assert_eq!(page.next_continuation_token, None);
    }

    #[test]
    fn parses_a_paginated_truncated_response() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>unidata-nexrad-level2</Name>
  <Prefix>2026/09/12/KTLX/</Prefix>
  <KeyCount>1</KeyCount>
  <MaxKeys>1</MaxKeys>
  <IsTruncated>true</IsTruncated>
  <NextContinuationToken>abc123token==</NextContinuationToken>
  <Contents>
    <Key>2026/09/12/KTLX/KTLX20260912_000110_V06</Key>
    <Size>6834037</Size>
  </Contents>
</ListBucketResult>"#;

        let page = parse_list_objects_v2(xml).unwrap();
        assert_eq!(
            page.keys,
            vec!["2026/09/12/KTLX/KTLX20260912_000110_V06".to_string()]
        );
        assert!(page.is_truncated);
        assert_eq!(
            page.next_continuation_token.as_deref(),
            Some("abc123token==")
        );
    }

    #[test]
    fn parses_an_empty_result() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>unidata-nexrad-level2</Name>
  <Prefix>2026/01/01/ZZZZ/</Prefix>
  <KeyCount>0</KeyCount>
  <MaxKeys>1000</MaxKeys>
  <IsTruncated>false</IsTruncated>
</ListBucketResult>"#;

        let page = parse_list_objects_v2(xml).unwrap();
        assert!(page.keys.is_empty());
        assert!(!page.is_truncated);
        assert_eq!(page.next_continuation_token, None);
    }

    #[test]
    fn rejects_truncated_xml_without_panicking() {
        // Cut off mid-tag, mid-document -- simulates a connection dropped
        // partway through the response body.
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Contents>
    <Key>2026/09/12/KTLX/KTLX20260912_000110_V06</Key>
    <Size>683"#;

        let result = parse_list_objects_v2(xml);
        assert!(
            result.is_err(),
            "truncated XML must be a structured error, not a panic"
        );
    }

    #[test]
    fn rejects_malformed_xml_without_panicking() {
        let xml = b"this is not xml at all <<<>>>";
        let result = parse_list_objects_v2(xml);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_mismatched_tags_without_panicking() {
        let xml = br#"<ListBucketResult><Contents><Key>foo</Contents></Key></ListBucketResult>"#;
        let result = parse_list_objects_v2(xml);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_utf8_without_panicking() {
        let xml: &[u8] =
            b"<ListBucketResult><Contents><Key>\xff\xfe</Key></Contents></ListBucketResult>";
        let result = parse_list_objects_v2(xml);
        assert!(result.is_err());
    }
}
