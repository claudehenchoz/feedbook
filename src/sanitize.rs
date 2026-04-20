use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use ammonia::Builder;
use regex::Regex;

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
        // picture: stripped (tag removed, <img> content kept) — EPUB3 readers don't
        // need responsive image selection, and <source srcset=...> elements inside
        // reference external URLs that EPUBCHECK flags as href-not-in-manifest.
        // source: also excluded; it is a void element so stripping it removes it fully.
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
    tag_attrs.insert("img",        ["src", "alt", "width", "height"].into_iter().collect());
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
    let sanitized = builder.clean(html).to_string();
    fixup_xhtml(&sanitized)
}

/// Post-sanitization pass to ensure XHTML5 / XML validity:
///   1. Self-close void elements  (<br> → <br />)
///   2. Replace named HTML entities with numeric character references
///      (&nbsp; → &#160; etc.) — only the 5 predefined XML entities
///      (&amp; &lt; &gt; &quot; &apos;) are valid in XML without a DTD.
///   3. Replace obsolete <tt> with <code> (not allowed in EPUB3).
///   4. Strip fragment-only hrefs from <a> tags (href="#...") to prevent
///      EPUBCHECK RSC-012 errors when the target id doesn't exist in the
///      extracted chapter (e.g. GitHub "Permalink" anchors whose id is
///      prefixed with "user-content-" while the href omits the prefix).
fn fixup_xhtml(html: &str) -> String {
    static VOID_RE:      OnceLock<Regex> = OnceLock::new();
    static ENTITY_RE:    OnceLock<Regex> = OnceLock::new();
    static TT_OPEN_RE:   OnceLock<Regex> = OnceLock::new();
    static A_TAG_RE:     OnceLock<Regex> = OnceLock::new();
    static FRAG_HREF_RE: OnceLock<Regex> = OnceLock::new();

    // Step 1: self-close void elements.
    // The attribute group requires leading whitespace, so <br/> is never
    // captured (/ is not \s). Already-closed <br /> is caught by the
    // ends_with("/>") guard.
    let void_re = VOID_RE.get_or_init(|| {
        Regex::new(
            r"(?i)<(area|base|br|col|embed|hr|img|input|link|meta|param|source|track|wbr)(\s[^>]*)?>",
        )
        .unwrap()
    });

    let html = void_re.replace_all(html, |caps: &regex::Captures| {
        let whole = &caps[0];
        if whole.ends_with("/>") {
            return whole.to_string(); // already self-closing
        }
        let tag = &caps[1];
        let attrs = caps.get(2).map_or("", |m| m.as_str());
        format!("<{}{} />", tag, attrs.trim_end())
    });

    // Step 2: named HTML entities → numeric character references.
    let entity_re = ENTITY_RE.get_or_init(|| {
        Regex::new(r"&([A-Za-z][A-Za-z0-9]*);").unwrap()
    });

    let html = entity_re
        .replace_all(&html, |caps: &regex::Captures| {
            match named_to_numeric(&caps[1]) {
                Some(numeric) => numeric.to_owned(),
                None => caps[0].to_owned(), // XML predefined or unknown — leave as-is
            }
        });

    // Step 3: replace obsolete <tt> with <code>.
    // html5ever normalises tag names to lowercase, so </tt> is sufficient.
    let tt_open = TT_OPEN_RE.get_or_init(|| {
        Regex::new(r"(?i)<tt(\b[^>]*)?>").unwrap()
    });
    let html = tt_open.replace_all(&html, |caps: &regex::Captures| {
        let attrs = caps.get(1).map_or("", |m| m.as_str());
        format!("<code{}>", attrs)
    });
    let html = if html.contains("</tt>") {
        std::borrow::Cow::Owned(html.replace("</tt>", "</code>"))
    } else {
        html
    };

    // Step 4: strip fragment-only hrefs from <a> tags.
    // After ammonia+html5ever serialisation, href only appears on <a> tags
    // and attribute values are double-quoted. A leading \s+ is always present
    // (html5ever puts a space before every attribute).
    let a_re = A_TAG_RE.get_or_init(|| Regex::new(r"(?i)<a\b[^>]*>").unwrap());
    let frag_re = FRAG_HREF_RE.get_or_init(|| {
        Regex::new(r##"\s+href="#[^"]*""##).unwrap()
    });
    a_re.replace_all(&html, |caps: &regex::Captures| {
        frag_re.replace(&caps[0], "").into_owned()
    })
    .into_owned()
}

/// Maps HTML4 named character references to their numeric equivalents.
/// Returns None for the 5 XML-predefined entities (valid without a DTD)
/// and for any name not in the HTML4 set.
fn named_to_numeric(name: &str) -> Option<&'static str> {
    match name {
        // XML predefined — valid in XML, leave unchanged
        "amp" | "lt" | "gt" | "quot" | "apos" => None,

        // ── Latin-1 supplement (U+00A0–U+00FF) ──────────────────────────────
        "nbsp"    => Some("&#160;"),
        "iexcl"   => Some("&#161;"),
        "cent"    => Some("&#162;"),
        "pound"   => Some("&#163;"),
        "curren"  => Some("&#164;"),
        "yen"     => Some("&#165;"),
        "brvbar"  => Some("&#166;"),
        "sect"    => Some("&#167;"),
        "uml"     => Some("&#168;"),
        "copy"    => Some("&#169;"),
        "ordf"    => Some("&#170;"),
        "laquo"   => Some("&#171;"),
        "not"     => Some("&#172;"),
        "shy"     => Some("&#173;"),
        "reg"     => Some("&#174;"),
        "macr"    => Some("&#175;"),
        "deg"     => Some("&#176;"),
        "plusmn"  => Some("&#177;"),
        "sup2"    => Some("&#178;"),
        "sup3"    => Some("&#179;"),
        "acute"   => Some("&#180;"),
        "micro"   => Some("&#181;"),
        "para"    => Some("&#182;"),
        "middot"  => Some("&#183;"),
        "cedil"   => Some("&#184;"),
        "sup1"    => Some("&#185;"),
        "ordm"    => Some("&#186;"),
        "raquo"   => Some("&#187;"),
        "frac14"  => Some("&#188;"),
        "frac12"  => Some("&#189;"),
        "frac34"  => Some("&#190;"),
        "iquest"  => Some("&#191;"),
        "Agrave"  => Some("&#192;"),
        "Aacute"  => Some("&#193;"),
        "Acirc"   => Some("&#194;"),
        "Atilde"  => Some("&#195;"),
        "Auml"    => Some("&#196;"),
        "Aring"   => Some("&#197;"),
        "AElig"   => Some("&#198;"),
        "Ccedil"  => Some("&#199;"),
        "Egrave"  => Some("&#200;"),
        "Eacute"  => Some("&#201;"),
        "Ecirc"   => Some("&#202;"),
        "Euml"    => Some("&#203;"),
        "Igrave"  => Some("&#204;"),
        "Iacute"  => Some("&#205;"),
        "Icirc"   => Some("&#206;"),
        "Iuml"    => Some("&#207;"),
        "ETH"     => Some("&#208;"),
        "Ntilde"  => Some("&#209;"),
        "Ograve"  => Some("&#210;"),
        "Oacute"  => Some("&#211;"),
        "Ocirc"   => Some("&#212;"),
        "Otilde"  => Some("&#213;"),
        "Ouml"    => Some("&#214;"),
        "times"   => Some("&#215;"),
        "Oslash"  => Some("&#216;"),
        "Ugrave"  => Some("&#217;"),
        "Uacute"  => Some("&#218;"),
        "Ucirc"   => Some("&#219;"),
        "Uuml"    => Some("&#220;"),
        "Yacute"  => Some("&#221;"),
        "THORN"   => Some("&#222;"),
        "szlig"   => Some("&#223;"),
        "agrave"  => Some("&#224;"),
        "aacute"  => Some("&#225;"),
        "acirc"   => Some("&#226;"),
        "atilde"  => Some("&#227;"),
        "auml"    => Some("&#228;"),
        "aring"   => Some("&#229;"),
        "aelig"   => Some("&#230;"),
        "ccedil"  => Some("&#231;"),
        "egrave"  => Some("&#232;"),
        "eacute"  => Some("&#233;"),
        "ecirc"   => Some("&#234;"),
        "euml"    => Some("&#235;"),
        "igrave"  => Some("&#236;"),
        "iacute"  => Some("&#237;"),
        "icirc"   => Some("&#238;"),
        "iuml"    => Some("&#239;"),
        "eth"     => Some("&#240;"),
        "ntilde"  => Some("&#241;"),
        "ograve"  => Some("&#242;"),
        "oacute"  => Some("&#243;"),
        "ocirc"   => Some("&#244;"),
        "otilde"  => Some("&#245;"),
        "ouml"    => Some("&#246;"),
        "divide"  => Some("&#247;"),
        "oslash"  => Some("&#248;"),
        "ugrave"  => Some("&#249;"),
        "uacute"  => Some("&#250;"),
        "ucirc"   => Some("&#251;"),
        "uuml"    => Some("&#252;"),
        "yacute"  => Some("&#253;"),
        "thorn"   => Some("&#254;"),
        "yuml"    => Some("&#255;"),

        // ── Latin extended / special ─────────────────────────────────────────
        "OElig"   => Some("&#338;"),
        "oelig"   => Some("&#339;"),
        "Scaron"  => Some("&#352;"),
        "scaron"  => Some("&#353;"),
        "Yuml"    => Some("&#376;"),
        "fnof"    => Some("&#402;"),
        "circ"    => Some("&#710;"),
        "tilde"   => Some("&#732;"),

        // ── Greek ────────────────────────────────────────────────────────────
        "Alpha"    => Some("&#913;"),
        "Beta"     => Some("&#914;"),
        "Gamma"    => Some("&#915;"),
        "Delta"    => Some("&#916;"),
        "Epsilon"  => Some("&#917;"),
        "Zeta"     => Some("&#918;"),
        "Eta"      => Some("&#919;"),
        "Theta"    => Some("&#920;"),
        "Iota"     => Some("&#921;"),
        "Kappa"    => Some("&#922;"),
        "Lambda"   => Some("&#923;"),
        "Mu"       => Some("&#924;"),
        "Nu"       => Some("&#925;"),
        "Xi"       => Some("&#926;"),
        "Omicron"  => Some("&#927;"),
        "Pi"       => Some("&#928;"),
        "Rho"      => Some("&#929;"),
        "Sigma"    => Some("&#931;"),
        "Tau"      => Some("&#932;"),
        "Upsilon"  => Some("&#933;"),
        "Phi"      => Some("&#934;"),
        "Chi"      => Some("&#935;"),
        "Psi"      => Some("&#936;"),
        "Omega"    => Some("&#937;"),
        "alpha"    => Some("&#945;"),
        "beta"     => Some("&#946;"),
        "gamma"    => Some("&#947;"),
        "delta"    => Some("&#948;"),
        "epsilon"  => Some("&#949;"),
        "zeta"     => Some("&#950;"),
        "eta"      => Some("&#951;"),
        "theta"    => Some("&#952;"),
        "iota"     => Some("&#953;"),
        "kappa"    => Some("&#954;"),
        "lambda"   => Some("&#955;"),
        "mu"       => Some("&#956;"),
        "nu"       => Some("&#957;"),
        "xi"       => Some("&#958;"),
        "omicron"  => Some("&#959;"),
        "pi"       => Some("&#960;"),
        "rho"      => Some("&#961;"),
        "sigmaf"   => Some("&#962;"),
        "sigma"    => Some("&#963;"),
        "tau"      => Some("&#964;"),
        "upsilon"  => Some("&#965;"),
        "phi"      => Some("&#966;"),
        "chi"      => Some("&#967;"),
        "psi"      => Some("&#968;"),
        "omega"    => Some("&#969;"),
        "thetasym" => Some("&#977;"),
        "upsih"    => Some("&#978;"),
        "piv"      => Some("&#982;"),

        // ── General punctuation / typography ────────────────────────────────
        "ensp"    => Some("&#8194;"),
        "emsp"    => Some("&#8195;"),
        "thinsp"  => Some("&#8201;"),
        "zwnj"    => Some("&#8204;"),
        "zwj"     => Some("&#8205;"),
        "lrm"     => Some("&#8206;"),
        "rlm"     => Some("&#8207;"),
        "ndash"   => Some("&#8211;"),
        "mdash"   => Some("&#8212;"),
        "lsquo"   => Some("&#8216;"),
        "rsquo"   => Some("&#8217;"),
        "sbquo"   => Some("&#8218;"),
        "ldquo"   => Some("&#8220;"),
        "rdquo"   => Some("&#8221;"),
        "bdquo"   => Some("&#8222;"),
        "dagger"  => Some("&#8224;"),
        "Dagger"  => Some("&#8225;"),
        "bull"    => Some("&#8226;"),
        "hellip"  => Some("&#8230;"),
        "permil"  => Some("&#8240;"),
        "prime"   => Some("&#8242;"),
        "Prime"   => Some("&#8243;"),
        "lsaquo"  => Some("&#8249;"),
        "rsaquo"  => Some("&#8250;"),
        "oline"   => Some("&#8254;"),
        "frasl"   => Some("&#8260;"),
        "euro"    => Some("&#8364;"),
        "image"   => Some("&#8465;"),
        "weierp"  => Some("&#8472;"),
        "real"    => Some("&#8476;"),
        "trade"   => Some("&#8482;"),
        "alefsym" => Some("&#8501;"),

        // ── Arrows ───────────────────────────────────────────────────────────
        "larr"  => Some("&#8592;"),
        "uarr"  => Some("&#8593;"),
        "rarr"  => Some("&#8594;"),
        "darr"  => Some("&#8595;"),
        "harr"  => Some("&#8596;"),
        "crarr" => Some("&#8629;"),
        "lArr"  => Some("&#8656;"),
        "uArr"  => Some("&#8657;"),
        "rArr"  => Some("&#8658;"),
        "dArr"  => Some("&#8659;"),
        "hArr"  => Some("&#8660;"),

        // ── Mathematical operators ───────────────────────────────────────────
        "forall" => Some("&#8704;"),
        "part"   => Some("&#8706;"),
        "exist"  => Some("&#8707;"),
        "empty"  => Some("&#8709;"),
        "nabla"  => Some("&#8711;"),
        "isin"   => Some("&#8712;"),
        "notin"  => Some("&#8713;"),
        "ni"     => Some("&#8715;"),
        "prod"   => Some("&#8719;"),
        "sum"    => Some("&#8721;"),
        "minus"  => Some("&#8722;"),
        "lowast" => Some("&#8727;"),
        "radic"  => Some("&#8730;"),
        "prop"   => Some("&#8733;"),
        "infin"  => Some("&#8734;"),
        "ang"    => Some("&#8736;"),
        "and"    => Some("&#8743;"),
        "or"     => Some("&#8744;"),
        "cap"    => Some("&#8745;"),
        "cup"    => Some("&#8746;"),
        "int"    => Some("&#8747;"),
        "there4" => Some("&#8756;"),
        "sim"    => Some("&#8764;"),
        "cong"   => Some("&#8773;"),
        "asymp"  => Some("&#8776;"),
        "ne"     => Some("&#8800;"),
        "equiv"  => Some("&#8801;"),
        "le"     => Some("&#8804;"),
        "ge"     => Some("&#8805;"),
        "sub"    => Some("&#8834;"),
        "sup"    => Some("&#8835;"),
        "nsub"   => Some("&#8836;"),
        "sube"   => Some("&#8838;"),
        "supe"   => Some("&#8839;"),
        "oplus"  => Some("&#8853;"),
        "otimes" => Some("&#8855;"),
        "perp"   => Some("&#8869;"),
        "sdot"   => Some("&#8901;"),
        "lceil"  => Some("&#8968;"),
        "rceil"  => Some("&#8969;"),
        "lfloor" => Some("&#8970;"),
        "rfloor" => Some("&#8971;"),
        "lang"   => Some("&#9001;"),
        "rang"   => Some("&#9002;"),
        "loz"    => Some("&#9674;"),
        "spades" => Some("&#9824;"),
        "clubs"  => Some("&#9827;"),
        "hearts" => Some("&#9829;"),
        "diams"  => Some("&#9830;"),

        _ => None, // unknown entity — leave as-is
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── named_to_numeric ──────────────────────────────────────────────────────

    #[test]
    fn named_to_numeric_nbsp() {
        assert_eq!(named_to_numeric("nbsp"), Some("&#160;"));
    }

    #[test]
    fn named_to_numeric_euro() {
        assert_eq!(named_to_numeric("euro"), Some("&#8364;"));
    }

    #[test]
    fn named_to_numeric_copy() {
        assert_eq!(named_to_numeric("copy"), Some("&#169;"));
    }

    #[test]
    fn named_to_numeric_xml_predefined_returns_none() {
        // XML predefined entities must not be converted
        for name in &["amp", "lt", "gt", "quot", "apos"] {
            assert_eq!(named_to_numeric(name), None, "expected None for &{};", name);
        }
    }

    #[test]
    fn named_to_numeric_unknown_returns_none() {
        assert_eq!(named_to_numeric("zzzznotanentity"), None);
    }

    // ── fixup_xhtml ───────────────────────────────────────────────────────────

    #[test]
    fn fixup_xhtml_self_closes_br() {
        let result = fixup_xhtml("<p>line one<br>line two</p>");
        assert!(result.contains("<br />"), "got: {}", result);
        assert!(!result.contains("<br>"));
    }

    #[test]
    fn fixup_xhtml_self_closes_img() {
        let result = fixup_xhtml(r#"<img src="x.jpg" alt="x">"#);
        assert!(result.contains("/>"), "got: {}", result);
    }

    #[test]
    fn fixup_xhtml_already_self_closed_unchanged() {
        let input = "<br />";
        let result = fixup_xhtml(input);
        // Should still be self-closed, not doubled
        assert!(result.contains("<br />"));
        assert!(!result.contains("<br />/>"));
    }

    #[test]
    fn fixup_xhtml_converts_named_entity_nbsp() {
        let result = fixup_xhtml("<p>hello&nbsp;world</p>");
        assert!(result.contains("&#160;"), "got: {}", result);
        assert!(!result.contains("&nbsp;"));
    }

    #[test]
    fn fixup_xhtml_leaves_xml_predefined_entities() {
        let result = fixup_xhtml("<p>a &amp; b &lt; c &gt; d</p>");
        assert!(result.contains("&amp;"));
        assert!(result.contains("&lt;"));
        assert!(result.contains("&gt;"));
    }

    #[test]
    fn fixup_xhtml_tt_to_code() {
        let result = fixup_xhtml("<p><tt>monospace</tt></p>");
        assert!(result.contains("<code>monospace</code>"), "got: {}", result);
        assert!(!result.contains("<tt>"));
    }

    #[test]
    fn fixup_xhtml_strips_fragment_only_href() {
        let result = fixup_xhtml(r##"<a href="#section">link</a>"##);
        assert!(!result.contains("href="), "got: {}", result);
        assert!(result.contains("<a>link</a>") || result.contains("<a "), "got: {}", result);
    }

    #[test]
    fn fixup_xhtml_preserves_full_href() {
        let result = fixup_xhtml(r##"<a href="https://example.com">link</a>"##);
        assert!(result.contains("href=\"https://example.com\""), "got: {}", result);
    }

    // ── sanitize_html ─────────────────────────────────────────────────────────

    #[test]
    fn sanitize_html_removes_script() {
        let sanitizer = build_sanitizer();
        let result = sanitize_html(&sanitizer, "<p>hello</p><script>alert('xss')</script>");
        assert!(!result.contains("<script>"), "got: {}", result);
        assert!(!result.contains("alert"), "got: {}", result);
        assert!(result.contains("<p>hello</p>"));
    }

    #[test]
    fn sanitize_html_removes_style_tag() {
        let sanitizer = build_sanitizer();
        let result = sanitize_html(&sanitizer, "<style>body{color:red}</style><p>text</p>");
        assert!(!result.contains("<style>"), "got: {}", result);
        assert!(result.contains("<p>text</p>"));
    }

    #[test]
    fn sanitize_html_strips_javascript_href() {
        let sanitizer = build_sanitizer();
        let result = sanitize_html(&sanitizer, r#"<a href="javascript:void(0)">click</a>"#);
        assert!(!result.contains("javascript:"), "got: {}", result);
    }

    #[test]
    fn sanitize_html_allows_basic_markup() {
        let sanitizer = build_sanitizer();
        let input = r#"<p>Hello <strong>world</strong></p><a href="https://example.com">link</a>"#;
        let result = sanitize_html(&sanitizer, input);
        assert!(result.contains("<p>"), "got: {}", result);
        assert!(result.contains("<strong>"), "got: {}", result);
        assert!(result.contains("href=\"https://example.com\""), "got: {}", result);
    }

    #[test]
    fn sanitize_html_strips_inline_style_attr() {
        let sanitizer = build_sanitizer();
        let result = sanitize_html(&sanitizer, r#"<p style="color:red">text</p>"#);
        assert!(!result.contains("style="), "got: {}", result);
        assert!(result.contains("<p>text</p>"));
    }
}
