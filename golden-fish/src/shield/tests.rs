use super::*;
fn valid(s: &str) -> bool {
    !s.contains(':') && !s.contains('{')
}
fn manager(rules: &str) -> FilterListManager {
    let mut m = FilterListManager::new();
    m.add(
        "test",
        "Test",
        "bundled:test",
        &alloc::format!("[Adblock Plus 2.0]\n{rules}"),
        valid,
    )
    .unwrap();
    m
}
fn decide(m: &FilterListManager, url: &str, document: &str, kind: ResourceType) -> Decision {
    m.evaluate(
        &CanonicalUrl::parse(url).unwrap(),
        &CanonicalUrl::parse(document).unwrap(),
        kind,
    )
}
#[test]
fn hostname_boundaries_and_canonical_authorities() {
    let m = manager("||ads.example.test^");
    for host in [
        "ads.example.test",
        "sub.ads.example.test",
        "ADS.EXAMPLE.TEST.",
    ] {
        assert_eq!(
            decide(
                &m,
                &alloc::format!("https://{host}/banner"),
                "https://page.test/",
                ResourceType::Image
            ),
            Decision::Block
        );
    }
    for url in [
        "https://ads.example.test.evil/",
        "https://badads.example.test/",
        "https://safe.test/ads.example.test/",
    ] {
        assert_eq!(
            decide(&m, url, "https://page.test/", ResourceType::Image),
            Decision::Allow
        );
    }
    for url in [
        "https://ads.test@safe.test/",
        "https://safe.test%2e.evil/",
        "https://a..test/",
        "https://[::1]/",
        "https://a.test:0/",
        "https://a.test\\@b.test/",
        "https://a.test/\n",
    ] {
        assert!(CanonicalUrl::parse(url).is_none(), "{url}");
    }
    assert_eq!(
        CanonicalUrl::parse("HTTPS://A.TEST.:443?q=1#frag")
            .unwrap()
            .url,
        "https://a.test/?q=1"
    );
    assert_eq!(
        CanonicalUrl::parse("http://127.000.0.1/").unwrap().host,
        "127.0.0.1"
    );
    let ip = manager("||127.000.0.1^\n||ADS.TEST.^$match-case");
    assert_eq!(
        decide(
            &ip,
            "http://127.0.0.1/",
            "https://page.test/",
            ResourceType::Image
        ),
        Decision::Block
    );
    assert_eq!(
        decide(
            &ip,
            "https://ads.test/",
            "https://page.test/",
            ResourceType::Image
        ),
        Decision::Block
    );
}
#[test]
fn wildcards_start_end_and_separator() {
    let cases = [
        (
            "/ads/*/banner^",
            "https://x.test/ads/a/b/banner?size=3",
            true,
        ),
        ("|https://x.test/a|", "https://x.test/a", true),
        ("|https://x.test/a|", "https://x.test/aa", false),
        ("banner^", "https://x.test/banner", true),
        ("banner^", "https://x.test/banner.png", false),
        ("ad*one*two|", "https://x.test/ad-one-two", true),
        ("ad*one*two|", "https://x.test/ad-one-two-extra", false),
        ("CASE$match-case", "https://x.test/case", false),
        ("CASE", "https://x.test/case", true),
    ];
    for (rule, url, blocked) in cases {
        assert_eq!(
            decide(&manager(rule), url, "https://x.test/", ResourceType::Image) == Decision::Block,
            blocked,
            "{rule} {url}"
        );
    }
}
#[test]
fn options_exceptions_and_top_level_navigation() {
    let m=manager("/ads/*$image,domain=page.test|~allow.page.test\n@@/ads/allowed.png$image\n/always$~stylesheet");
    assert_eq!(
        decide(
            &m,
            "https://cdn.test/ads/a.png",
            "https://page.test/",
            ResourceType::Image
        ),
        Decision::Block
    );
    assert_eq!(
        decide(
            &m,
            "https://cdn.test/ads/allowed.png",
            "https://page.test/",
            ResourceType::Image
        ),
        Decision::Exception
    );
    for (site, kind) in [
        ("https://allow.page.test/", ResourceType::Image),
        ("https://page.test/", ResourceType::Script),
        ("https://other.test/", ResourceType::Image),
        ("https://page.test/", ResourceType::Document),
    ] {
        assert_eq!(
            decide(&m, "https://cdn.test/ads/a.png", site, kind),
            Decision::Allow
        );
    }
    assert_eq!(
        decide(
            &m,
            "https://cdn.test/always",
            "https://page.test/",
            ResourceType::Stylesheet
        ),
        Decision::Allow
    );
}
#[test]
fn public_suffixes_preserve_first_party_and_private_tenant_boundaries() {
    let m = manager("/asset$third-party\n/first$~third-party");
    let cases = [
        ("cdn.example.co.uk", "www.example.co.uk", false),
        ("other.co.uk", "example.co.uk", true),
        ("one.github.io", "two.github.io", true),
        ("a.one.github.io", "one.github.io", false),
        ("127.0.0.1", "127.0.0.2", true),
        ("a.city.kawasaki.jp", "city.kawasaki.jp", false),
    ];
    for (target, site, third_party) in cases {
        assert_eq!(
            decide(
                &m,
                &alloc::format!("https://{target}/asset"),
                &alloc::format!("https://{site}/"),
                ResourceType::Image
            ) == Decision::Block,
            third_party,
            "{target} {site}"
        );
    }
    assert_eq!(
        decide(
            &m,
            "https://cdn.example.test/first",
            "https://www.example.test/",
            ResourceType::Image
        ),
        Decision::Block
    );
}
#[test]
fn unsupported_rules_are_never_broadened() {
    let rules="/good$image\n/secret$redirect=noop\n/secret$unknown\n/secret$domain=\n/secret$important\n/secret$document\n/secret$domain=*.test\n@@/secret$unknown\n/ad.*/\nexample.test#?#div:has(.ad)\n##div:has(.ad)\n##+js(foo)\n/secret$~third-party,third-party";
    let m = manager(rules);
    assert_eq!(
        decide(
            &m,
            "https://x.test/secret",
            "https://x.test/",
            ResourceType::Image
        ),
        Decision::Allow
    );
    assert_eq!(m.lists[0].compiled.diagnostics.supported, 1);
    assert_eq!(m.lists[0].compiled.diagnostics.unsupported, 12);
    assert!(compile("[Adblock Plus 2.0]\n!#if unknown\n/secret\n!#endif", valid).is_err());
    let mut m = manager("/retained");
    let original = m.lists[0].compiled.clone();
    assert!(m
        .replace("test", "[Adblock Plus 2.0]\n/only$unknown", 0, valid)
        .is_err());
    assert!(Arc::ptr_eq(&original, &m.lists[0].compiled));
}
#[test]
fn cosmetics_domain_exceptions_and_hiding_switches() {
    let m=manager("##.ad\npage.test##.sponsor\npage.test#@#.ad\n~safe.test##.generic\n@@||nohide.test^$elemhide\n@@||nogeneric.test^$generichide\nnogeneric.test##.specific");
    assert_eq!(
        m.cosmetic_selectors(&CanonicalUrl::parse("https://page.test/").unwrap()),
        vec![".generic", ".sponsor"]
    );
    assert!(m
        .cosmetic_selectors(&CanonicalUrl::parse("https://nohide.test/").unwrap())
        .is_empty());
    assert_eq!(
        m.cosmetic_selectors(&CanonicalUrl::parse("https://nogeneric.test/").unwrap()),
        vec![".specific"]
    );
    assert_eq!(
        m.cosmetic_selectors(&CanonicalUrl::parse("https://safe.test/").unwrap()),
        vec![".ad"]
    );
}
#[test]
fn settings_roundtrip_toggles_and_exact_site_exceptions() {
    let mut m = manager("/ads/*");
    assert!(m.settings.set_site_exception("WWW.Site.TEST.", true));
    let doc = CanonicalUrl::parse("https://www.site.test/").unwrap();
    let req = CanonicalUrl::parse("https://cdn.test/ads/x").unwrap();
    assert_eq!(m.evaluate(&req, &doc, ResourceType::Image), Decision::Allow);
    assert_eq!(
        decide(
            &m,
            &req.url,
            "https://sub.www.site.test/",
            ResourceType::Image
        ),
        Decision::Block
    );
    m.settings.disabled_lists.insert("test".into());
    assert_eq!(
        decide(&m, &req.url, "https://other.test/", ResourceType::Image),
        Decision::Allow
    );
    assert_eq!(
        FilterSettings::decode(&m.settings.encode().unwrap()),
        Some(m.settings.clone())
    );
    for invalid in [
        "shield-v1\nenabled=0\nsite=foo@evil",
        "shield-v1\nenabled=0\nunknown=1",
        "shield-v2\nenabled=0",
    ] {
        assert!(FilterSettings::decode(invalid).is_none());
    }
}
#[test]
fn atomic_replacement_and_cross_list_exception() {
    let mut m = manager("/ads/*\n/ads/*");
    let old = m.lists[0].compiled.clone();
    let revision = m.revision;
    assert!(m
        .replace("test", "<html>not a list</html>", 1, valid)
        .is_err());
    assert!(Arc::ptr_eq(&old, &m.lists[0].compiled));
    assert_eq!(revision, m.revision);
    m.add(
        "allow",
        "Allow",
        "bundled:allow",
        "[Adblock Plus 2.0]\n@@/ads/*",
        valid,
    )
    .unwrap();
    assert_eq!(
        decide(
            &m,
            "https://x.test/ads/a",
            "https://page.test/",
            ResourceType::Image
        ),
        Decision::Exception
    );
    m.replace("test", "[Adblock Plus 2.0]\n/other/*", 123, valid)
        .unwrap();
    assert!(m.revision > revision);
    assert_eq!(m.lists[0].last_successful_update, Some(123));
    assert_eq!(old.diagnostics.duplicates, 1);
}
#[test]
fn bounded_adversarial_input_and_diagnostics() {
    assert_eq!(
        compile(&"x".repeat(MAX_LIST_BYTES + 1), valid).unwrap_err(),
        CompileError::TooLarge
    );
    let input = alloc::format!(
        "[Adblock Plus 2.0]\n/good\n{}\n{}",
        "x".repeat(MAX_LINE_BYTES + 1),
        "/bad$unknown\n".repeat(500)
    );
    let c = compile(&input, valid).unwrap();
    assert!(c.diagnostics.samples.len() <= 32);
    assert_eq!(c.diagnostics.supported, 1);
    let repeated = manager("aaaa*");
    let url = CanonicalUrl::parse(&alloc::format!("https://a.test/{}", "a".repeat(7000))).unwrap();
    assert_eq!(repeated.lists[0].compiled.candidates(&url).len(), 1);
    let m = manager(&alloc::format!("|{}z|", "*a".repeat(250)));
    let decision = m.evaluate(
        &url,
        &CanonicalUrl::parse("https://a.test/").unwrap(),
        ResourceType::Image,
    );
    assert!(matches!(decision, Decision::Allow | Decision::WorkLimit));
}

#[test]
fn immutable_source_compilation_and_nested_domain_precedence() {
    const SOURCE: &str = "[Adblock Plus 2.0]\n||ads.test^$domain=example.test|~news.example.test|ads.news.example.test\nexample.test,~news.example.test,ads.news.example.test##.advert";
    let static_rules = compile_static(SOURCE, valid).unwrap();
    let dynamic_rules = compile(SOURCE, valid).unwrap();
    assert!(matches!(static_rules.network[0].pattern, Cow::Borrowed(_)));
    assert!(matches!(
        static_rules.network[0].domains.raw,
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        static_rules.cosmetic[0].selector,
        Cow::Borrowed(_)
    ));
    assert!(matches!(dynamic_rules.cosmetic[0].selector, Cow::Owned(_)));
    for (host, expected) in [
        ("example.test", true),
        ("news.example.test", false),
        ("ads.news.example.test", true),
        ("unrelated.test", false),
    ] {
        assert_eq!(static_rules.network[0].domains.matches(host), expected);
        assert_eq!(static_rules.cosmetic[0].domains.matches(host), expected);
    }
    assert!(static_rules.compiled_bytes() < dynamic_rules.compiled_bytes());
}

#[test]
fn element_hiding_exception_subsumes_generic_exception_in_both_orders() {
    for options in ["elemhide,generichide", "generichide,elemhide"] {
        let m = manager(&alloc::format!(
            "@@||page.test^${options}\n##.ad\npage.test##.site-ad"
        ));
        assert!(m
            .cosmetic_selectors(&CanonicalUrl::parse("https://page.test/").unwrap())
            .is_empty());
    }
}
