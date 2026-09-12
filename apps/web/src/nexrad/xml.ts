// Parsing for S3 `ListObjectsV2` XML responses, using the browser's built-in
// `DOMParser` (no XML-parsing npm dependency needed in the browser the way
// `crates/radar-cache/src/xml.rs` needs `quick-xml` natively).
//
// This is a deliberate TypeScript re-implementation of that Rust module's
// logic (see its doc comment for the exact response shape reference) --
// `radar-cache` itself is `tokio`/`reqwest`-coupled and native-only, so it
// cannot be reused directly from a browser bundle (per this task's brief).
// Only the fields discovery needs are extracted: each `<Contents><Key>`
// value, and the pagination fields `<IsTruncated>` / `<NextContinuationToken>`.
//
// Per GLOBAL_CONTRACT ("remote data is unreliable and untrusted" / "no
// uncontrolled panics on malformed input"), a malformed/truncated response
// never throws an uncaught exception from deep inside parsing -- callers get
// a thrown `Error` with a clear message (DOMParser itself never throws; it
// produces a `<parsererror>` document instead, which is explicitly checked
// for below), never a silent wrong answer.

export interface ListObjectsPage {
  /** Every `<Contents><Key>` value in this page, in document order. */
  keys: string[];
  isTruncated: boolean;
  nextContinuationToken: string | null;
}

/**
 * Parse one page of a `ListObjectsV2` XML response body.
 *
 * @throws {Error} if the body is not well-formed XML, or does not look
 * like a `ListBucketResult` document at all.
 */
export function parseListObjectsV2(xmlText: string): ListObjectsPage {
  const doc = new DOMParser().parseFromString(xmlText, "application/xml");

  const parserError = doc.querySelector("parsererror");
  if (parserError) {
    throw new Error(`malformed ListObjectsV2 XML response: ${parserError.textContent ?? "unknown parse error"}`);
  }

  const root = doc.documentElement;
  if (!root || root.localName !== "ListBucketResult") {
    throw new Error(
      `unexpected XML document root: expected <ListBucketResult>, got ${root ? `<${root.localName}>` : "(none)"}`,
    );
  }

  // Only direct children of <Contents> elements count as object keys --
  // mirrors `xml.rs`'s `in_contents` tracking, so a <Key> appearing
  // somewhere else in the document (not that ListObjectsV2 ever produces
  // one, but the input is untrusted) is not mistaken for an object.
  const keys: string[] = [];
  for (const contents of root.getElementsByTagName("Contents")) {
    const keyEl = contents.getElementsByTagName("Key")[0];
    if (keyEl?.textContent != null) {
      keys.push(keyEl.textContent);
    }
  }

  const isTruncatedText = root.getElementsByTagName("IsTruncated")[0]?.textContent ?? "false";
  const isTruncated = isTruncatedText.trim().toLowerCase() === "true";

  const tokenEl = root.getElementsByTagName("NextContinuationToken")[0];
  const nextContinuationToken = tokenEl?.textContent ?? null;

  return { keys, isTruncated, nextContinuationToken };
}
