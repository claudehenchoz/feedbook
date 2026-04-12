use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use ammonia::Builder;

pub fn build_sanitizer() -> Arc<Builder<'static>> {
    let mut b = Builder::default();

    // --- Tags ---
    // Start from ammonia's conservative default, then add HTML5 structural/semantic
    // elements that are valid EPUB content but not in ammonia's default allowlist.
    b.add_tags(&[
        "section", "article", "aside", "nav", "header", "footer", "main",
        "figure", "figcaption", "hgroup",
        "mark", "time", "ruby", "rt", "rp", "rb", "rtc", "bdi", "wbr",
        "details", "summary",
        "picture", "source",
        "audio", "video", "track",
        "svg",
    ]);

    // Tags to remove entirely (content and all):
    b.rm_tags(&["script", "style", "iframe", "object", "embed", "form",
                "input", "button", "select", "textarea", "link", "meta"]);

    // --- Generic attributes (allowed on any tag) ---
    b.add_generic_attributes(&[
        "id", "class", "title", "lang", "xml:lang", "dir",
        "role",
        "epub:type",
        "translate", "hidden",
    ]);

    // aria-* and data-* via prefix matching:
    b.add_generic_attribute_prefixes(&["aria-", "data-"]);

    // --- Per-tag attributes ---
    let mut tag_attrs: HashMap<&str, HashSet<&str>> = HashMap::new();
    tag_attrs.insert("a",          ["href", "hreflang", "type"].into_iter().collect());
    tag_attrs.insert("img",        ["src", "alt", "width", "height", "srcset", "sizes", "longdesc"].into_iter().collect());
    tag_attrs.insert("source",     ["src", "srcset", "type", "media", "sizes"].into_iter().collect());
    tag_attrs.insert("audio",      ["src", "controls", "preload"].into_iter().collect());
    tag_attrs.insert("video",      ["src", "controls", "preload", "poster", "width", "height"].into_iter().collect());
    tag_attrs.insert("track",      ["src", "kind", "srclang", "label", "default"].into_iter().collect());
    tag_attrs.insert("th",         ["colspan", "rowspan", "scope", "headers"].into_iter().collect());
    tag_attrs.insert("td",         ["colspan", "rowspan", "headers"].into_iter().collect());
    tag_attrs.insert("col",        ["span"].into_iter().collect());
    tag_attrs.insert("colgroup",   ["span"].into_iter().collect());
    tag_attrs.insert("ol",         ["start", "reversed", "type"].into_iter().collect());
    tag_attrs.insert("li",         ["value"].into_iter().collect());
    tag_attrs.insert("time",       ["datetime"].into_iter().collect());
    tag_attrs.insert("q",          ["cite"].into_iter().collect());
    tag_attrs.insert("blockquote", ["cite"].into_iter().collect());
    b.tag_attributes(tag_attrs);

    // --- URL schemes ---
    // Drop javascript:, data: in hrefs by only allowing safe ones.
    b.url_schemes(["http", "https", "mailto", "tel"].into_iter().collect());
    // Allow relative URLs so internal links survive:
    b.url_relative(ammonia::UrlRelative::PassThrough);
    // Preserve the original `rel` attribute on <a> rather than forcing
    // "noopener noreferrer". ammonia owns this attribute and panics if
    // it also appears in tag_attributes.
    b.link_rel(None);

    // Strip inline style by default — EPUBs should use a separate stylesheet.
    // (ammonia strips `style` attribute by default; don't add it back.)

    Arc::new(b)
}

pub fn sanitize_html(builder: &Builder<'static>, html: &str) -> String {
    builder.clean(html).to_string()
}
