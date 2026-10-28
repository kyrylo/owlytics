use owlytics::{
    acquisition::{Attribution, Campaign, ClickId, channel, source_name},
    analytics::{self, Collected, Event},
    db::Database,
};

#[test]
fn source_names_consolidate_tags_domains_and_android_apps_without_merging_lookalikes() {
    for (raw, expected) in [
        ("FB-ads", "Facebook"),
        ("facebook_feed_ad", "Facebook"),
        ("GoogleAds", "Google"),
        ("adwords", "Google"),
        ("yt_ads", "YouTube"),
        ("IG", "Instagram"),
        ("HN", "Hacker News"),
        ("l.facebook.com", "Facebook"),
        ("www.google.com.ph", "Google"),
        ("gemini.google.com", "Gemini"),
        ("mail.google.com", "Gmail"),
        (
            "android-app://com.google.android.googlequicksearchbox",
            "Google",
        ),
        ("google.com.evil.example", "google.com.evil.example"),
        ("notgoogle.com", "notgoogle.com"),
        ("FACEBOOKbad", "FACEBOOKbad"),
        ("My Newsletter", "My Newsletter"),
        ("猫", "猫"),
        ("", ""),
    ] {
        assert_eq!(source_name(raw), expected, "{raw}");
    }
    assert_eq!(
        analytics::referrer(
            "android-app://com.google.android.googlequicksearchbox/search?secret=x#fragment",
            "example.com"
        ),
        "android-app://com.google.android.googlequicksearchbox/search"
    );
    assert_eq!(
        analytics::source("android-app://unknown.example/path", "example.com"),
        "android-app://unknown.example"
    );
    for url in [
        "https://example.com/path",
        "http://localhost/",
        "data:text/plain,private",
    ] {
        assert!(analytics::source(url, "example.com").is_empty());
    }
}

#[test]
fn channel_definitions_cover_paid_organic_and_competing_rules() {
    for (source, medium, campaign, expected) in [
        ("", "", "", "Direct"),
        ("unknown.example", "", "", "Referral"),
        ("", "referral", "", "Referral"),
        ("", "organic", "", "Organic Search"),
        ("google", "", "", "Organic Search"),
        ("bing", "CPC", "", "Paid Search"),
        ("adwords", "", "", "Paid Search"),
        ("googleads", "", "", "Paid Search"),
        ("Google", "(gclid)", "", "Paid Search"),
        ("Bing", "(msclkid)", "", "Paid Search"),
        ("fb-ads", "", "", "Paid Social"),
        ("facebook_feed_ad", "", "", "Paid Social"),
        ("instagram", "paid_social", "", "Paid Social"),
        ("GitHub", "", "", "Organic Social"),
        ("", "social network", "", "Organic Social"),
        ("yt-ads", "", "", "Paid Video"),
        ("youtube", "ppc", "", "Paid Video"),
        ("youtube", "", "", "Organic Video"),
        ("", "short-video", "", "Organic Video"),
        ("Amazon", "", "", "Organic Shopping"),
        ("", "ppc", "eshop", "Paid Shopping"),
        ("", "", "shopping", "Organic Shopping"),
        ("", "", "workshop", "Direct"),
        ("", "cross-network", "", "Cross-network"),
        ("Google", "cpc", "fall-cross-network", "Cross-network"),
        ("", "display", "", "Display"),
        ("", "cpm", "", "Display"),
        ("Google", "cpm", "", "Paid Search"),
        ("unknown.example", "retargeting", "", "Paid Other"),
        ("ChatGPT", "", "", "AI Assistants"),
        ("claude.ai", "", "", "AI Assistants"),
        ("perplexity.ai", "", "", "AI Assistants"),
        ("gemini.google.com", "", "", "AI Assistants"),
        ("Gmail", "", "", "Email"),
        ("Weekly Newsletter", "", "", "Email"),
        ("", "E-Mail", "", "Email"),
        ("", "affiliate", "", "Affiliates"),
        ("", "audio", "", "Audio"),
        ("SMS", "", "", "SMS"),
        ("", "sms", "", "SMS"),
        ("Firebase", "", "", "Mobile Push Notifications"),
        ("", "webpush", "", "Mobile Push Notifications"),
    ] {
        assert_eq!(
            channel(&source_name(source), source, medium, campaign),
            expected,
            "{source}/{medium}/{campaign}"
        );
    }
}

#[test]
fn explicit_tags_take_precedence_and_click_ids_can_only_be_parameter_names() {
    let tags: Campaign = serde_json::from_str(
        r#"{"source":"FB-Ads","medium":"CPC","name":"Autumn","content":"猫","term":"Rust"}"#,
    )
    .unwrap();
    let a = Attribution::resolve("google.com", &tags);
    assert_eq!(
        (&*a.source, &*a.utm_source, &*a.utm_medium, a.channel),
        ("Facebook", "FB-Ads", "CPC", "Paid Social")
    );
    assert_eq!(
        (&*a.utm_campaign, &*a.utm_content, &*a.utm_term),
        ("Autumn", "猫", "Rust")
    );
    for (referrer, click, expected) in [
        ("google.com", ClickId::Gclid, "(gclid)"),
        ("bing.com", ClickId::Msclkid, "(msclkid)"),
    ] {
        let a = Attribution::resolve(
            referrer,
            &Campaign {
                click_id: Some(click),
                ..Default::default()
            },
        );
        assert_eq!(a.utm_medium, expected);
        assert_eq!(a.channel, "Paid Search");
        let a = Attribution::resolve(
            referrer,
            &Campaign {
                medium: Some(String::new()),
                click_id: Some(click),
                ..Default::default()
            },
        );
        assert_eq!(a.utm_medium, "");
        assert_eq!(a.channel, "Paid Search");
        assert_eq!(a.click_id, Some(click.label()));
    }
    let a = Attribution::resolve(
        "google.com",
        &Campaign {
            source: Some(String::new()),
            ..Default::default()
        },
    );
    assert_eq!((a.source.as_str(), a.channel), ("", "Direct"));
    let a = Attribution::resolve(
        "",
        &Campaign {
            click_id: Some(ClickId::Gclid),
            ..Default::default()
        },
    );
    assert_eq!((a.source.as_str(), a.utm_medium.as_str()), ("", ""));
    for json in [
        r#"{"click_id":"secret-user-id"}"#,
        r#"{"gclid":"secret"}"#,
        r#"{"url":"https://private.example"}"#,
    ] {
        assert!(serde_json::from_str::<Campaign>(json).is_err());
    }
    for value in [
        "\0secret".into(),
        "a\u{85}b".into(),
        "🦉".repeat(600),
        "\"".repeat(1100),
    ] {
        assert!(
            !Campaign {
                source: Some(value),
                ..Default::default()
            }
            .valid()
        );
    }
    assert!(
        Campaign {
            source: Some("a".repeat(2000)),
            ..Default::default()
        }
        .valid()
    );
}

const START: i64 = 1793059200;
const UA: &str = "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36";
async fn fixture() -> Database {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    db
}
async fn view(db: &Database, id: u128, second: i64, ip: &str, tags: Option<Campaign>) -> Collected {
    analytics::collect(
        db,
        Event {
            domain: "example.com".into(),
            path: format!("/{id}"),
            referrer: "https://www.google.com/search?secret=x".into(),
            id: format!("{id:032x}"),
            campaign: tags,
        },
        ip.parse().unwrap(),
        UA.into(),
        move || START + second,
    )
    .await
    .unwrap()
}
fn facebook() -> Option<Campaign> {
    Some(Campaign {
        source: Some("FB-Ads".into()),
        medium: Some("CPC".into()),
        name: Some("Autumn".into()),
        ..Default::default()
    })
}

#[tokio::test]
async fn session_attribution_and_summaries_match_independent_groups_after_every_transition() {
    let db = fixture().await;
    for (id, second, ip, tags) in [
        (1, 0, "192.0.2.1", facebook()),
        (2, 30, "192.0.2.1", Some(Campaign::default())),
        (2, 60, "192.0.2.1", facebook()), // retry cannot change anything
        (3, 1890, "192.0.2.1", Some(Campaign::default())),
        (4, 3750, "192.0.2.1", facebook()),
        (5, 3710, "192.0.2.2", facebook()),
        (6, 3720, "192.0.2.3", None), // old tracker, followed by new tracker
        (7, 3730, "192.0.2.3", facebook()),
        (8, 86400, "192.0.2.1", Some(Campaign::default())), // daily rotation
    ] {
        assert_eq!(
            view(&db, id, second, ip, tags).await,
            if id == 2 && second == 60 {
                Collected::Duplicate
            } else {
                Collected::Stored
            }
        );
        db.read(|c| {
            for key in ["channel", "utm_source", "utm_medium", "utm_campaign", "utm_content", "utm_term"] {
                let read = |sql: &str| -> rusqlite::Result<Vec<(String,String,i64,i64,i64,i64,i64)>> {
                    c.prepare(sql)?.query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))?.collect()
                };
                let oracle = read(&format!("SELECT s.day,a.{key},COUNT(DISTINCT s.visitor),SUM(s.pageviews),COUNT(*),SUM(s.pageviews=1),SUM(s.last_at-s.started_at) FROM sessions s JOIN session_acquisition a ON a.session_id=s.id GROUP BY s.day,a.{key} ORDER BY 1,2"))?;
                let summary = read(&format!("SELECT day,value,visitors,pageviews,visits,bounces,duration FROM daily_session_reports WHERE dimension='{key}' ORDER BY 1,2"))?;
                assert_eq!(summary,oracle,"{key}");
                let events = c.prepare(&format!("SELECT day,value,visitors,pageviews FROM daily_reports WHERE dimension='{key}' ORDER BY 1,2"))?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                assert_eq!(events,oracle.into_iter().map(|(day,name,visitors,views,_,_,_)|(day,name,visitors,views)).collect::<Vec<_>>());
            }
            Ok(())
        }).await.unwrap();
    }
    db.read(|c| {
        assert_eq!(c.query_row("SELECT source FROM events WHERE id=2",[],|r|r.get::<_,String>(0))?,"Facebook");
        assert_eq!(c.query_row("SELECT utm_source FROM session_acquisition WHERE session_id=1",[],|r|r.get::<_,String>(0))?,"FB-Ads");
        assert_eq!(c.query_row("SELECT COUNT(*) FROM session_acquisition WHERE session_id=6",[],|r|r.get::<_,i64>(0))?,0);
        assert_eq!(c.query_row("SELECT visitors FROM daily_reports WHERE day='2026-10-27' AND dimension='channel' AND value='Paid Social'",[],|r|r.get::<_,i64>(0))?,2);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM session_acquisition",[],|r|r.get::<_,i64>(0))?,5);
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn unknown_legacy_campaigns_and_invalid_new_payloads_do_not_create_known_direct_rows() {
    let db = fixture().await;
    assert_eq!(view(&db, 1, 0, "192.0.2.1", None).await, Collected::Stored);
    assert_eq!(
        view(
            &db,
            2,
            1,
            "192.0.2.2",
            Some(Campaign {
                source: Some("bad\0value".into()),
                ..Default::default()
            })
        )
        .await,
        Collected::Invalid
    );
    db.read(|c| {
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))?,
            1
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM session_acquisition", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM daily_reports WHERE dimension='channel'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            0
        );
        Ok(())
    })
    .await
    .unwrap();
    // A failed metadata insert must roll back the pageview, session and summaries.
    db.call(|c| c.execute_batch("CREATE TRIGGER reject_campaign BEFORE INSERT ON session_acquisition BEGIN SELECT RAISE(ABORT,'test failure'); END;")).await.unwrap();
    let result = analytics::collect(
        &db,
        Event {
            domain: "example.com".into(),
            path: "/".into(),
            referrer: String::new(),
            id: format!("{:032x}", 3),
            campaign: Some(Campaign::default()),
        },
        "192.0.2.3".parse().unwrap(),
        UA.into(),
        || START + 2,
    )
    .await;
    assert!(result.is_err());
    db.read(|c| {
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))?,
            1
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0))?,
            1
        );
        assert_eq!(
            c.query_row(
                "SELECT SUM(pageviews) FROM daily_reports WHERE dimension='total'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            1
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn campaign_reports_use_distinct_tagged_denominators_and_share_filter_semantics() {
    use owlytics::{
        analytics::{ReportQuery, ReportScope},
        filters::{Filter, Operator},
    };
    let db = fixture().await;
    let tags = |source: &str, medium: &str, name: &str| {
        Some(Campaign {
            source: Some(source.into()),
            medium: Some(medium.into()),
            name: Some(name.into()),
            ..Default::default()
        })
    };
    for (id, second, ip, campaign) in [
        (1, 0, "192.0.2.1", tags("FB-Ads", "CPC", "A%_")),
        (2, 10, "192.0.2.1", tags("ignored", "ignored", "ignored")),
        (3, 1870, "192.0.2.1", tags("fb-ads", "cpc", "B")),
        (4, 1900, "192.0.2.2", tags("Google", "cpc", "B")),
        (5, 2000, "192.0.2.3", Some(Campaign::default())),
        (6, 2100, "192.0.2.4", None),
        (7, 86400, "192.0.2.1", tags("FB-Ads", "CPC", "A%_")),
    ] {
        assert_eq!(view(&db, id, second, ip, campaign).await, Collected::Stored);
    }
    let base = ReportQuery {
        from: Some("2026-10-27".into()),
        to: Some("2026-10-28".into()),
        interval: Some("day".into()),
        ..Default::default()
    };
    let range = base.range("2026-10-28".parse().unwrap()).unwrap();
    let summary = analytics::report(&db, 1, range.clone(), base.clone())
        .await
        .unwrap();
    assert_eq!((summary.visitors, summary.pageviews), (5, 7));
    let raw = analytics::report(
        &db,
        1,
        range.clone(),
        ReportQuery {
            browser: Some("Chrome".into()),
            ..base.clone()
        },
    )
    .await
    .unwrap();
    for (key, _) in owlytics::acquisition::DIMENSIONS {
        assert_eq!(
            summary.acquisition_rows(key),
            raw.acquisition_rows(key),
            "{key}"
        );
        assert!(summary.acquisition_incomplete.contains(*key));
        assert!(raw.acquisition_incomplete.contains(*key));
    }
    let a = summary
        .acquisition_rows("utm_campaign")
        .iter()
        .find(|r| r.name == "A%_")
        .unwrap();
    let b = summary
        .acquisition_rows("utm_campaign")
        .iter()
        .find(|r| r.name == "B")
        .unwrap();
    assert_eq!(
        (
            a.visitors,
            a.pageviews,
            a.visits,
            a.bounces,
            a.duration,
            a.percentage
        ),
        (2, 3, 2, 1, 10, Some((2, 3)))
    );
    assert_eq!(
        (b.visitors, b.pageviews, b.percentage),
        (2, 2, Some((2, 3)))
    );
    assert_eq!(summary.acquisition_rows("utm_campaign").len(), 2);
    let social = summary
        .acquisition_rows("channel")
        .iter()
        .find(|r| r.name == "Paid Social")
        .unwrap();
    assert_eq!(social.percentage, Some((2, 4)));
    assert!(summary.acquisition_rows("utm_content").is_empty());
    let filter = |key: &str, op, values: &[&str]| Filter {
        key: key.into(),
        op,
        values: values.iter().map(|v| (*v).into()).collect(),
    };
    for (filters, visitors, views) in [
        (
            vec![
                filter("utm_medium", Operator::Is, &["CPC", "cpc"]),
                filter("utm_campaign", Operator::Contains, &["B"]),
            ],
            2,
            2,
        ),
        (
            vec![filter("utm_campaign", Operator::Contains, &["%_"])],
            2,
            3,
        ),
        (
            vec![filter("utm_campaign", Operator::ContainsNot, &["B"])],
            3,
            4,
        ),
        (vec![filter("utm_medium", Operator::Is, &[""])], 1, 1),
        (
            vec![filter("channel", Operator::IsNot, &["Paid Social"])],
            2,
            2,
        ),
    ] {
        let query = ReportQuery {
            filters: filters.clone(),
            ..base.clone()
        };
        let actual = analytics::report(&db, 1, range.clone(), query.clone())
            .await
            .unwrap();
        assert_eq!(
            (actual.visitors, actual.pageviews),
            (visitors, views),
            "{filters:?}"
        );
        let comparison = analytics::report(
            &db,
            1,
            range.clone(),
            ReportQuery {
                scope: ReportScope::Timeline,
                ..query.clone()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (
                comparison.visitors,
                comparison.pageviews,
                comparison.totals.visits
            ),
            (actual.visitors, actual.pageviews, actual.totals.visits)
        );
        let hourly = analytics::report(
            &db,
            1,
            range.clone(),
            ReportQuery {
                interval: Some("hour".into()),
                ..query
            },
        )
        .await
        .unwrap();
        assert_eq!(
            hourly
                .timeline
                .iter()
                .map(|bucket| bucket.pageviews)
                .sum::<i64>(),
            views
        );
    }
    let options = analytics::report(
        &db,
        1,
        range.clone(),
        ReportQuery {
            scope: ReportScope::Suggestions,
            filter: Some("utm_medium".into()),
            detail: Some("utm_medium".into()),
            ..base.clone()
        },
    )
    .await
    .unwrap();
    assert!(
        options
            .acquisition_rows("utm_medium")
            .iter()
            .any(|row| row.name.is_empty() && row.visitors == 1)
    );
    let detail = analytics::report(
        &db,
        1,
        range,
        ReportQuery {
            scope: ReportScope::Dashboard,
            sources_tab: Some("utm_campaign".into()),
            detail: Some("utm_campaign".into()),
            sort: Some("name".into()),
            order: Some("desc".into()),
            search: Some("B".into()),
            ..base
        },
    )
    .await
    .unwrap();
    assert_eq!(detail.detail_total, 1);
    assert_eq!(
        detail.acquisition_rows("utm_campaign")[0].percentage,
        Some((2, 3)),
        "Search must not change the tagged audience denominator"
    );
    assert!(detail.acquisition_rows("utm_medium").is_empty());
}
