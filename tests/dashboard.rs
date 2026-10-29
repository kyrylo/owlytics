mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use owlytics::{auth, db::Database};
use tower::ServiceExt;

async fn fixture() -> (axum::Router, Database, String, String) {
    let db = Database::open(":memory:").unwrap();
    let config = common::config();
    let token = auth::random_token().unwrap();
    let csrf = auth::random_token().unwrap();
    let token_hash = auth::digest(&token);
    let credential_hash = auth::digest(&config.password_hash);
    let csrf_copy = csrf.clone();
    db.call(move |c| {
        c.execute(
            "INSERT INTO owner_sessions VALUES(?1,?2,?3,?4)",
            rusqlite::params![token_hash, csrf_copy, credential_hash, auth::now() + 3600],
        )
    })
    .await
    .unwrap();
    (
        owlytics::router(db.clone(), config),
        db,
        format!("__Host-owlytics_session={token}"),
        csrf,
    )
}
async fn body(response: axum::response::Response) -> String {
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}
async fn request(
    app: &axum::Router,
    path: &str,
    cookie: &str,
    form: Option<&[(&str, &str)]>,
) -> axum::response::Response {
    let mut builder = Request::builder().uri(path).header("cookie", cookie);
    let payload = if let Some(form) = form {
        builder = builder
            .method("POST")
            .header("origin", "https://stats.example.com")
            .header("content-type", "application/x-www-form-urlencoded");
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form.iter().copied())
            .finish()
    } else {
        String::new()
    };
    app.clone()
        .oneshot(builder.body(Body::from(payload)).unwrap())
        .await
        .unwrap()
}

fn report_link(html: &str, marker: &str) -> String {
    html.split("<a ")
        .map(|part| part.split("</a>").next().unwrap())
        .find(|part| part.contains(marker))
        .unwrap_or_else(|| panic!("Missing link {marker}"))
        .split("href=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .replace("&amp;", "&")
}

#[tokio::test]
async fn grouped_filter_forms_show_selection_keep_state_and_share_export_counts() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
        for (index,browser,country,path) in [(1,"Chrome","US","/a"),(2,"Safari","US","/b"),(3,"Firefox","US","/c"),(4,"Opera","PH","/d")] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793059200,'2026-10-27',?1,?1,?2,'',?3,?4,'Desktop')",
                rusqlite::params![i128::to_be_bytes(index).as_slice(),path,country,browser])?;
        }
        Ok(())
    }).await.unwrap();
    let target = "/sites/1/filter?from=2026-10-27&to=2026-10-27&country=US&metric=pageviews&compare=off&detail=page&sort=name&order=asc&filter=browser&filter_op=is&value=Chrome&value=Safari";
    assert_eq!(
        request(&app, target, "", None).await.status(),
        StatusCode::SEE_OTHER
    );
    let response = request(&app, target, &cookie, None).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()["location"].to_str().unwrap().to_owned();
    let html = body(request(&app, &location, &cookie, None).await).await;
    assert!(html.contains("<span>Unique visitors</span><strong>2</strong>"));
    assert!(html.contains("Chrome, Safari"));
    assert!(html.contains("Edit Browser operator"));
    assert!(html.contains("Remove browser filter"));
    assert!(html.contains("value=\"Chrome\" checked"));
    assert!(html.contains("value=\"Safari\" checked"));
    assert!(
        html.contains("value=\"Firefox\""),
        "Suggestions ignore the edited browser filter"
    );
    assert!(
        !html.contains("value=\"Opera\""),
        "Suggestions retain the country filter"
    );
    assert!(location.contains("detail=page&sort=name&order=asc"));
    let done = report_link(&html, "class=\"filter-done\"");
    assert!(!done.contains("filter="));
    assert!(done.contains("filters="));
    let export = report_link(&html, "Export stats");
    let mut zip = zip_body(request(&app, &export, &cookie, None).await).await;
    let rows = csv_rows(&mut zip, "summary.csv");
    assert_eq!(&rows[1][3], "2");
    assert_eq!(&rows[1][5], "2");
    let removed = report_link(&html, "Remove browser filter");
    let removed_html = body(request(&app, &removed, &cookie, None).await).await;
    assert!(removed_html.contains("<span>Unique visitors</span><strong>3</strong>"));
    assert!(!removed_html.contains("Remove browser filter"));
    assert!(removed_html.contains("Remove country filter"));
    let cleared = report_link(&html, "class=\"clear-filters\"");
    let cleared_html = body(request(&app, &cleared, &cookie, None).await).await;
    assert!(cleared_html.contains("<span>Unique visitors</span><strong>4</strong>"));
    assert!(!cleared.contains("filters="));
    assert!(cleared.contains("metric=pageviews"));
    assert!(cleared.contains("compare=off"));

    let excluded = target.replace("filter_op=is&", "filter_op=is_not&");
    let response = request(&app, &excluded, &cookie, None).await;
    let html = body(
        request(
            &app,
            response.headers()["location"].to_str().unwrap(),
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(html.contains("<span>Unique visitors</span><strong>1</strong>"));
    assert!(html.contains(">is not</a>"));
    db.call(|c| {
        for (index,browser,country) in [(10,"Chrome","US"),(11,"Firefox","US"),(12,"Safari","PH")] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1792972800,'2026-10-26',?1,?1,'/a','',?2,?3,'Desktop')", rusqlite::params![i128::to_be_bytes(index).as_slice(),country,browser])?;
        }
        Ok(())
    }).await.unwrap();
    let comparison = location.replace(
        "compare=off",
        "compare=custom&compare_from=2026-10-26&compare_to=2026-10-26&interval=day",
    );
    let html = body(request(&app, &comparison, &cookie, None).await).await;
    assert!(html.contains("data-value=\"2\""));
    assert!(
        html.contains("data-previous-value=\"1\""),
        "Comparison retains both the multi-browser and country filters"
    );
}

#[tokio::test]
async fn grouped_filter_inputs_are_validated_escaped_and_empty_selection_is_removable() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    for (name, value) in [
        ("filters", "not-json"),
        (
            "filters",
            "[{\"key\":\"page\",\"op\":\"bad\",\"values\":[\"x\"]}]",
        ),
        ("filters", "[{\"key\":\"page\",\"values\":[]}]"),
        ("filter", "unknown"),
        ("filter_mode", "unknown"),
        ("filter_offset", "1000001"),
        ("filter_search", "bad\ntext"),
    ] {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair(name, value)
            .finish();
        for path in ["/sites/1", "/sites/1/filter", "/sites/1/export"] {
            assert_eq!(
                request(&app, &format!("{path}?{query}"), &cookie, None)
                    .await
                    .status(),
                StatusCode::BAD_REQUEST,
                "{path} {query}"
            );
        }
    }
    let values = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("filter", "page"),
            ("filter_op", "contains"),
            ("term", "/<img src=x onerror=alert(1)>"),
        ])
        .finish();
    let response = request(&app, &format!("/sites/1/filter?{values}"), &cookie, None).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let html = body(
        request(
            &app,
            response.headers()["location"].to_str().unwrap(),
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(!html.contains("<img src=x"));
    assert!(html.contains("/&lt;img src=x onerror=alert(1)&gt;"));
    assert!(html.contains("Remove page filter"));
    let response = request(
        &app,
        "/sites/1/filter?filter=page&filter_op=is",
        &cookie,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let html = body(
        request(
            &app,
            response.headers()["location"].to_str().unwrap(),
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(html.contains("Select…"));
    let done = report_link(&html, "class=\"filter-done\"");
    assert!(!done.contains("filter="));
    assert!(!done.contains("filters="));
}

#[tokio::test]
async fn date_defaults_use_site_history_and_invalid_comparisons_are_recoverable() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute(
            "INSERT INTO sites VALUES(1,'example.com','Pacific/Kiritimati',0)",
            [],
        )
    })
    .await
    .unwrap();
    let site = owlytics::analytics::site(&db, 1).await.unwrap().unwrap();
    let today = site.today();
    let html = body(request(&app, "/sites/1", &cookie, None).await).await;
    assert!(html.contains("</svg><span>Today</span>"));
    let target = report_link(&html, "Show pageviews");
    assert!(target.contains(&format!("from={today}&to={today}")));
    let yesterday = today.pred_opt().unwrap().to_string();
    db.call(move |c| {
        c.execute(
            "INSERT INTO import_batches VALUES(1,1,?1,?1,0)",
            [&yesterday],
        )?;
        c.execute(
            "INSERT INTO import_daily VALUES(1,?1,1,10,20)",
            [&yesterday],
        )
    })
    .await
    .unwrap();
    let html = body(request(&app, "/sites/1", &cookie, None).await).await;
    assert!(html.contains("</svg><span>Last 28 days</span>"));
    let target = report_link(&html, "Show pageviews");
    assert!(target.contains(&format!(
        "from={}&to={}",
        today - chrono::Duration::days(28),
        today.pred_opt().unwrap()
    )));
    for query in [
        "compare=unknown",
        "compare_match=unknown",
        "compare=custom",
        "compare=custom&compare_from=2025-01-01",
        "compare=custom&compare_to=2025-01-01",
        "compare=custom&compare_from=2025-02-29&compare_to=2025-03-01",
        "compare=custom&compare_from=2025-02-01&compare_to=2025-01-01",
        "compare=custom&compare_from=2000-01-01&compare_to=2025-01-01",
        "compare=custom&compare_from=0000-01-01&compare_to=0000-01-01",
    ] {
        let response = request(&app, &format!("/sites/1?{query}"), &cookie, None).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        let html = body(response).await;
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains("Back to dashboard"));
        let response = request(&app, &format!("/sites/1/export?{query}"), &cookie, None).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "export {query}");
    }
}

#[tokio::test]
async fn custom_comparison_counts_and_csv_use_the_same_calendar_intervals() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0);
            INSERT INTO import_batches VALUES(1,1,'2025-01-01','2025-01-02',0);
            INSERT INTO import_daily VALUES(1,'2025-01-01',1,10,20),(1,'2025-01-02',1,20,40);")?;
        c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1738411200,'2025-02-01',randomblob(16),randomblob(16),'/','','US','Chrome','Desktop')", [])
    }).await.unwrap();
    let query = "from=2025-02-01&to=2025-02-01&compare=custom&compare_from=2025-01-01&compare_to=2025-01-02&compare_match=weekday";
    let html = body(request(&app, &format!("/sites/1?{query}"), &cookie, None).await).await;
    assert!(html.contains("title=\"Jan 1 – Jan 2, 2025\""));
    assert!(html.contains("data-date=\"Sat, 01 Feb 2025\" data-value=\"1\""));
    assert!(html.contains("data-previous-date=\"Wed, 01 Jan 2025\" data-previous-value=\"10\""));
    assert!(html.contains("data-previous-date=\"Thu, 02 Jan 2025\" data-previous-value=\"20\""));
    assert_eq!(html.matches("class=\"chart-hit\"").count(), 2);
    assert!(
        !html.contains("Match day of week"),
        "Custom dates are exact"
    );
    let mut archive =
        zip_body(request(&app, &format!("/sites/1/export?{query}"), &cookie, None).await).await;
    let rows = csv_rows(&mut archive, "visitors.csv");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1][0], "2025-02-01");
    let html = body(
        request(
            &app,
            &format!("/sites/1?{query}&browser=Chrome"),
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(html.contains("Comparison shows collected data only. Historical data is excluded."));
}

#[tokio::test]
async fn locations_drill_down_and_clear_children_while_preserving_report_state() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c|{
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)",[])?;
        for (country,region,city) in [("US","US: US-CA","US: 5391959"),("US","US: US-NY","US: 5128581"),("DE","DE: DE-BE","DE: Berlin")] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device,region,city) VALUES(1,1793059200,'2026-10-27',randomblob(16),randomblob(16),'/','Google',?1,'Chrome','Desktop',?2,?3)",rusqlite::params![country,region,city])?;
        } Ok(())
    }).await.unwrap();
    let html=body(request(&app,"/sites/1?from=2026-10-27&to=2026-10-27&browser=Chrome&source=Google&metric=pageviews&interval=day&compare=off",&cookie,None).await).await;
    assert!(html.contains("class=\"country-map\""));
    let map = html
        .split("<figure class=\"country-map\">")
        .nth(1)
        .unwrap()
        .split("</figure>")
        .next()
        .unwrap();
    assert!(
        map.len() < 40_000,
        "Report refreshes must not embed static map geometry"
    );
    assert!(map.contains("href=\"/assets/country-shapes.") && map.contains(".svg#US\""));
    let country = report_link(&html, "data-country=\"US\"");
    for pair in [
        "locations_tab=region",
        "country=US",
        "browser=Chrome",
        "source=Google",
        "metric=pageviews",
        "interval=day",
        "compare=off",
    ] {
        assert!(country.contains(pair), "{pair}: {country}");
    }
    let html = body(request(&app, &country, &cookie, None).await).await;
    assert!(html.contains("panel-region"));
    assert!(html.contains("data-filter-key=\"country\""));
    assert!(html.contains("title=\"United States\""));
    let region = report_link(&html, "title=\"California ·");
    assert!(region.contains("locations_tab=city"));
    assert!(region.contains("country=US"));
    let html = body(request(&app, &region, &cookie, None).await).await;
    assert!(html.contains("panel-city"));
    assert!(html.contains("data-filter-key=\"region\""));
    assert!(html.contains("title=\"California\""));
    assert!(html.contains("San Francisco, California"));
    assert!(!html.contains("New York City"));
    let city = report_link(&html, "title=\"San Francisco, California ·");
    let html = body(request(&app, &city, &cookie, None).await).await;
    assert!(html.contains("data-filter-key=\"city\""));
    assert!(html.contains("title=\"San Francisco, California\""));
    let cleared = report_link(&html, "aria-label=\"Remove country filter\"");
    for key in ["country=", "region=", "city="] {
        assert!(!cleared.contains(key), "{cleared}");
    }
    assert!(cleared.contains("locations_tab=map"));
    assert!(cleared.contains("source=Google"));
    let html = body(request(&app, &cleared, &cookie, None).await).await;
    assert!(html.contains("data-country=\"DE\""));
    let global = body(
        request(
            &app,
            "/sites/1?from=2026-10-27&to=2026-10-27&locations_tab=region",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    let region = report_link(&global, "title=\"California ·");
    assert!(region.contains("country=US"));
    assert!(region.contains("locations_tab=city"));
}

#[tokio::test]
async fn country_map_includes_low_ranked_countries_and_reports_unmapped_traffic() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c|{
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)",[])?;
        for code in include_str!("../assets/country-shapes.tsv").lines().map(|line|line.split_once('\t').unwrap().0).chain(["","AQ"]) {
            for _ in 0..if code=="US" || code.is_empty() || code=="AQ" {1} else {2} {
                c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793059200,'2026-10-27',randomblob(16),randomblob(16),'/','',?1,'Chrome','Desktop')",[code])?;
            }
        } Ok(())
    }).await.unwrap();
    let html = body(
        request(
            &app,
            "/sites/1?from=2026-10-27&to=2026-10-27",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(html.matches("data-country=").count(), 236);
    assert!(html.contains("aria-label=\"United States · 1 visitor\""));
    assert!(html.contains("Not on map: 1 visitor"));
    assert!(html.contains("data-country=\"SG\""));
}

#[tokio::test]
async fn imported_city_names_sort_search_export_and_preserve_location_ids() {
    use std::io::Read;
    let (app, db, cookie, _) = fixture().await;
    db.call(|c|{
        c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0); INSERT INTO import_batches VALUES(1,1,'2026-10-01','2026-10-01',0); INSERT INTO import_daily VALUES(1,'2026-10-01',1,100,100);")?;
        for (city,count) in [("US: 5391959",3),("US: 4887398",2),("US: 4930956",1)] {
            c.execute("INSERT INTO import_breakdowns VALUES(1,'2026-10-01','city',?1,1,?2)",rusqlite::params![city,count])?;
            c.execute("INSERT INTO import_metrics VALUES(1,'2026-10-01','city',?1,1,?2,?2,0,0,NULL)",rusqlite::params![city,count])?;
        }Ok(())
    }).await.unwrap();
    let base = "/sites/1?from=2026-10-01&to=2026-10-01&detail=city";
    for sort in ["name", "visitors", "percentage"] {
        let response = request(
            &app,
            &format!("{base}&sort={sort}&order=asc"),
            &cookie,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "sort={sort}");
        let html = body(response).await;
        assert!(
            html.find("Boston, Massachusetts").unwrap() < html.find("Chicago, Illinois").unwrap()
        );
        assert!(
            html.find("Chicago, Illinois").unwrap()
                < html.find("San Francisco, California").unwrap()
        );
        assert!(html.contains("<td>3</td><td>0%</td><td>0s</td>"));
        assert!(html.contains("https://www.geonames.org/"));
        assert!(
            report_link(&html, "title=\"San Francisco, California\"")
                .contains("city=US%3A+5391959")
        );
    }
    let compact = body(
        request(
            &app,
            &base.replace("detail=city", "locations_tab=city"),
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(compact.contains("class=\"row-percentage\">50.0%"));
    let html = body(request(&app, &format!("{base}&search=california"), &cookie, None).await).await;
    assert!(html.contains("San Francisco, California"));
    assert!(!html.contains("Chicago, Illinois"));
    assert!(html.contains("1–1 of 1"));
    let mut archive = zip_body(
        request(
            &app,
            "/sites/1/export?from=2026-10-01&to=2026-10-01",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    let cities = csv_rows(&mut archive, "cities.csv");
    assert_eq!(
        cities[0],
        ["city", "visitors", "bounce_rate", "visit_duration"]
    );
    assert_eq!(
        cities[1],
        ["San Francisco, California", "3", "0.00", "0.00"]
    );
    let mut credits = String::new();
    archive
        .by_name("readme.txt")
        .unwrap()
        .read_to_string(&mut credits)
        .unwrap();
    assert!(credits.contains("GeoNames"));
    assert!(credits.contains("https://creativecommons.org/licenses/by/4.0/"));
    db.read(|c| {
        assert_eq!(
            c.query_row(
                "SELECT value FROM import_breakdowns WHERE pageviews=3",
                [],
                |r| r.get::<_, String>(0)
            )?,
            "US: 5391959"
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn owner_can_add_multiple_sites_and_install_the_tracker() {
    let (app, db, cookie, csrf) = fixture().await;
    for path in ["/", "/sites/new", "/sites/1", "/sites/1/settings"] {
        assert_eq!(
            request(&app, path, "", None).await.status(),
            StatusCode::SEE_OTHER
        );
    }
    let denied = request(
        &app,
        "/sites",
        &cookie,
        Some(&[
            ("domain", "example.com"),
            ("timezone", "UTC"),
            ("csrf", "invalid"),
        ]),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let invalid = request(
        &app,
        "/sites",
        &cookie,
        Some(&[
            ("domain", "<script>alert(1)</script>"),
            ("timezone", "Mars/Olympus"),
            ("csrf", &csrf),
        ]),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let html = body(invalid).await;
    assert!(!html.contains("<script>alert"));
    assert!(html.contains("&lt;script&gt;"));
    for (domain, target) in [
        ("Example.COM", "/sites/1/settings"),
        ("other.example", "/sites/2/settings"),
    ] {
        let response = request(
            &app,
            "/sites",
            &cookie,
            Some(&[
                ("domain", domain),
                ("timezone", "Asia/Manila"),
                ("csrf", &csrf),
            ]),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()["location"], target);
    }
    let duplicate = request(
        &app,
        "/sites",
        &cookie,
        Some(&[
            ("domain", "example.com"),
            ("timezone", "UTC"),
            ("csrf", &csrf),
        ]),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    let install = body(request(&app, "/sites/1/settings", &cookie, None).await).await;
    assert!(install.contains("&lt;script defer data-domain=&quot;example.com&quot; src=&quot;https://stats.example.com/script.js&quot;&gt;&lt;/script&gt;"));
    let sites = owlytics::analytics::sites(&db).await.unwrap();
    assert_eq!(sites.len(), 2);
    assert_eq!(sites[0].domain, "example.com");
    assert_eq!(
        request(&app, "/sites/999", &cookie, None).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn dashboard_escapes_collected_text_and_applies_date_and_dimension_filters() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
        c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,0,'2026-10-27',zeroblob(16),zeroblob(16),?1,'','PH','Safari','Mobile')", ["/<script>alert(1)</script>"])?;
        Ok(())
    }).await.unwrap();
    let response = request(
        &app,
        "/sites/1?from=2026-10-01&to=2026-10-31",
        &cookie,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body(response).await;
    assert!(!html.contains("<script>alert"));
    assert!(html.contains("/&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html.contains("Direct / None"));
    assert!(html.contains("country=PH"));
    assert!(html.contains("current visitors"));
    let filtered = body(
        request(
            &app,
            "/sites/1?from=2026-10-01&to=2026-10-31&browser=Chrome",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(filtered.contains("<span>Unique visitors</span><strong>0</strong>"));
    assert!(filtered.contains("Remove browser filter"));
    for path in [
        "/sites/1?from=invalid",
        "/sites/1?from=2026-10-31&to=2026-10-01",
        "/sites/1?from=1900-01-01&to=2026-01-01",
    ] {
        assert_eq!(
            request(&app, path, &cookie, None).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn project_index_shows_recent_traffic_and_links_to_reports_and_settings() {
    let (app, db, cookie, _) = fixture().await;
    let now = chrono::Utc::now();
    let day = now.date_naive().to_string();
    db.call(move |c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0),(2,'quiet.example','UTC',0)", [])?;
        c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,?2,zeroblob(16),zeroblob(16),'/','','','Safari','Desktop')", rusqlite::params![now.timestamp(),day])?;
        Ok(())
    }).await.unwrap();
    let response = request(&app, "/", &cookie, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body(response).await;
    assert_eq!(html.matches("class=\"site-card\"").count(), 2);
    assert_eq!(html.matches("class=\"site-sparkline\"").count(), 2);
    assert!(html.contains("visitors in last 24h"));
    assert!(html.contains("title=\"1 visitors\">1</strong>"));
    assert!(html.contains("href=\"/sites/1?period=24h"));
    assert!(html.contains("aria-label=\"Settings for example.com\""));
    assert!(!html.contains("theme.js"));
    assert!(!html.contains("data-theme"));
    assert!(html.contains("src=\"/assets/favicons.js\""));
    assert!(html.contains("data-favicon=\"/sites/1/favicon\""));
}

#[tokio::test]
async fn metrics_preserve_navigation_and_empty_reports_offer_the_right_next_step() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    let html = body(request(&app, "/sites/1", &cookie, None).await).await;
    assert!(html.contains("Waiting for your first pageview"));
    assert!(html.contains("Install tracker"));
    assert!(html.contains("report-grid"));
    db.call(|c|c.execute_batch("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,unixepoch('2026-10-27'),'2026-10-27',zeroblob(16),zeroblob(16),'/pricing','','PH','Safari','Mobile'),(1,unixepoch('2026-10-27'),'2026-10-27',zeroblob(16),randomblob(16),'/pricing','','PH','Safari','Mobile');")).await.unwrap();
    let html=body(request(&app,"/sites/1?from=2026-10-27&to=2026-10-27&metric=pageviews&page=%2Fpricing&browser=Safari",&cookie,None).await).await;
    assert!(html.contains("<span>Unique visitors</span><strong>1</strong>"));
    assert!(html.contains("<span>Total pageviews</span><strong>2</strong>"));
    assert!(html.contains("Tue, 27 Oct 2026: 2 pageviews"));
    assert!(html.contains("aria-label=\"Pageviews from"));
    assert!(html.contains("name=\"metric\" value=\"pageviews\""));
    assert!(html.contains("metric=pageviews&amp;page=%2Fpricing&amp;browser=Safari"));
    let clear = url::Url::parse(&format!(
        "https://stats.example.com{}",
        report_link(&html, "class=\"clear-filters\"")
    ))
    .unwrap();
    assert_eq!(
        clear
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>(),
        [
            ("from", "2026-10-27"),
            ("to", "2026-10-27"),
            ("metric", "pageviews"),
            ("interval", "day")
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
    );
    let empty = body(
        request(
            &app,
            "/sites/1?from=2026-10-27&to=2026-10-27&metric=pageviews&page=%2Fmissing",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(empty.contains("No matching pageviews"));
    assert!(empty.contains("Clear filters"));
    assert_eq!(empty.matches(">Clear filters</a>").count(), 1);
    assert!(empty.contains("report-grid"));
    let outside = body(
        request(
            &app,
            "/sites/1?from=2020-01-01&to=2020-01-02",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(outside.contains("No pageviews in this period"));
    assert!(outside.contains("Last 28 days"));
    let invalid = request(&app, "/sites/1?from=wrong", &cookie, None).await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let html = body(invalid).await;
    assert!(html.contains("role=\"alert\""));
    assert!(html.contains("Back to dashboard"));
    assert!(html.contains("class=\"nav\""));
    assert_eq!(
        request(&app, "/sites/1?metric=invalid", &cookie, None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn historical_visitors_without_pageviews_still_have_a_chart() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0); INSERT INTO import_batches VALUES(1,1,'2026-10-01','2026-10-01',0); INSERT INTO import_daily VALUES(1,'2026-10-01',1,5,0);")).await.unwrap();
    for metric in ["visitors", "pageviews"] {
        let response = request(
            &app,
            &format!("/sites/1?from=2026-10-01&to=2026-10-01&metric={metric}"),
            &cookie,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = body(response).await;
        assert!(html.contains("<span>Unique visitors</span><strong>5</strong>"));
        assert!(html.contains("<span>Total pageviews</span><strong>0</strong>"));
        assert!(html.contains("class=\"chart\""));
        assert!(!html.contains("No pageviews in this period"));
        assert!(!html.contains("class=\"empty-report\""));
    }
}

async fn export_fixture(db: &Database) {
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
        for i in 0..106_i64 {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793059200,'2026-10-27',?1,?1,?2,'=1+1','PH','Safari','Mobile')", rusqlite::params![(i as u128).to_be_bytes().to_vec(), format!("/page/{i:03}")])?;
        }
        Ok(())
    }).await.unwrap();
}

async fn zip_body(response: axum::response::Response) -> zip::ZipArchive<std::io::Cursor<Vec<u8>>> {
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap()
}
fn csv_rows(
    archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
    file: &str,
) -> Vec<Vec<String>> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(archive.by_name(file).unwrap())
        .records()
        .map(|row| row.unwrap().iter().map(str::to_owned).collect())
        .collect()
}

#[tokio::test]
async fn minute_calendar_charts_and_exports_preserve_dst_counts_and_import_coverage() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','America/New_York',0)", [])?;
        for instant in ["2025-11-02T05:50:00Z", "2025-11-02T06:10:00Z"] {
            let stamp = chrono::DateTime::parse_from_rfc3339(instant).unwrap().timestamp();
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,'2025-11-02',zeroblob(16),randomblob(16),'/','Google','US','Chrome','Desktop')", [stamp])?;
        }
        c.execute_batch("INSERT INTO import_batches VALUES(1,1,'2025-11-01','2025-11-01',0); INSERT INTO import_daily VALUES(1,'2025-11-01',1,3,7);")
    }).await.unwrap();
    let path = "/sites/1?period=day&date=2025-11-02&interval=minute&metric=pageviews&compare=off";
    let html = body(request(&app, path, &cookie, None).await).await;
    assert_eq!(html.matches("class=\"chart-hit\"").count(), 1500);
    assert!(html.contains("01:50 EDT") && html.contains("01:10 EST"));
    let response = request(
        &app,
        &path.replace("/sites/1?", "/sites/1/export?"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut archive = zip_body(response).await;
    let rows = csv_rows(&mut archive, "visitors.csv");
    assert_eq!(rows.len(), 1501);
    assert_eq!(
        rows[1..]
            .iter()
            .map(|row| row[3].parse::<i64>().unwrap())
            .sum::<i64>(),
        2
    );
    for (stamp, views) in [
        ("2025-11-02T01:50:00-04:00", "1"),
        ("2025-11-02T01:10:00-05:00", "1"),
    ] {
        assert_eq!(rows.iter().find(|row| row[0] == stamp).unwrap()[3], views);
    }
    // An imported comparison can only support daily values, on both sides.
    let html = body(
        request(
            &app,
            &path.replace("compare=off", "compare=previous&compare_match=date"),
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(html.matches("class=\"chart-hit\"").count(), 1);
    assert!(!html.contains(">Minutes<") && !html.contains(">Hours<"));
    assert!(html.contains("Previous: 7"));
    // Preset changes drop an interval so the destination can use its own choice.
    assert!(!report_link(&html, ">Last 28 days").contains("interval="));
}

#[tokio::test]
async fn rolling_headlines_and_csv_use_complete_visits_for_averages() {
    use chrono::Timelike;
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        let now = chrono::Utc::now();
        // Keep the visit away from local midnight; this regression isolates a
        // rolling-window cut from the separately tracked identity-rotation work.
        let zone = if (1..23).contains(&now.hour()) { chrono_tz::UTC } else { chrono_tz::Pacific::Honolulu };
        c.execute("INSERT INTO sites VALUES(1,'example.com',?1,0)", [zone.name()])?;
        let boundary = now.timestamp() - 86400;
        for (person, offset) in [(0_i64,-120),(0,120),(0,240),(1,300)] {
            let stamp = boundary + offset;
            let day = chrono::DateTime::from_timestamp(stamp,0).unwrap().with_timezone(&zone).date_naive().to_string();
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,?2,?3,randomblob(16),'/','Google','US','Chrome','Desktop')",
                rusqlite::params![stamp,day,(person as i128).to_be_bytes().as_slice()])?;
        }
        Ok(())
    }).await.unwrap();
    for filter in ["", "&source=Google"] {
        let path = format!("/sites/1?period=24h&metric=views_per_visit&compare=off{filter}");
        let html = body(request(&app, &path, &cookie, None).await).await;
        assert!(html.contains("<span>Total pageviews</span><strong>3</strong>"));
        assert!(html.contains("<span>Views per visit</span><strong>2.00</strong>"));
        let mut zip =
            zip_body(request(&app, &report_link(&html, "Export stats"), &cookie, None).await).await;
        assert_eq!(
            &csv_rows(&mut zip, "summary.csv")[1][3..],
            ["2", "2", "3", "2.00", "50.00", "180.0"]
        );
    }
}

#[tokio::test]
async fn native_and_imported_means_stay_weighted_in_charts_and_exports() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0);
            INSERT INTO import_batches VALUES(1,1,'2026-10-26','2026-10-26',0);
            INSERT INTO import_daily VALUES(1,'2026-10-26',1,2,10);
            INSERT INTO import_breakdowns VALUES(1,'2026-10-26','source','Google',1,10);
            INSERT INTO import_metrics(site_id,day,dimension,value,batch_id,visitors,visits,bounces,duration)
              VALUES(1,'2026-10-26','total','',1,2,2,1,600),(1,'2026-10-26','source','Google',1,2,2,1,600);")?;
        for (person, seconds) in [(0_i128,0),(0,60),(1,120)] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793059200+?1,'2026-10-27',?2,randomblob(16),'/','Google','US','Chrome','Desktop')",rusqlite::params![seconds,person.to_be_bytes().as_slice()])?;
        }
        Ok(())
    }).await.unwrap();
    for filter in ["", "&source=Google"] {
        for interval in ["week", "month"] {
            let path = format!(
                "/sites/1?from=2026-10-26&to=2026-10-27&interval={interval}&metric=views_per_visit&compare=off{filter}"
            );
            let html = body(request(&app, &path, &cookie, None).await).await;
            assert_eq!(html.matches("class=\"chart-hit\"").count(), 1);
            let point = html
                .split("class=\"chart-hit\"")
                .nth(1)
                .unwrap()
                .split('>')
                .next()
                .unwrap();
            assert!(point.contains("data-value=\"3.25\""), "{point}");
            let mut zip =
                zip_body(request(&app, &report_link(&html, "Export stats"), &cookie, None).await)
                    .await;
            assert_eq!(
                &csv_rows(&mut zip, "summary.csv")[1][3..],
                ["4", "4", "13", "3.25", "50.00", "165.0"]
            );
            let days = csv_rows(&mut zip, "visitors.csv");
            assert_eq!(days.len(), 2);
            assert_eq!(
                days[1][0],
                if interval == "week" {
                    "2026-10-26"
                } else {
                    "2026-10-01"
                }
            );
            assert_eq!(&days[1][1..], ["4", "4", "13", "3.25", "50.00", "165.0"]);
        }
        let mut zip = zip_body(
            request(
                &app,
                &format!(
                    "/sites/1/export?from=2026-10-26&to=2026-10-27&interval=day&compare=off{filter}"
                ),
                &cookie,
                None,
            )
            .await,
        )
        .await;
        let days = csv_rows(&mut zip, "visitors.csv");
        assert_eq!(days.len(), 3);
        assert_eq!(&days[1][1..], ["2", "2", "10", "5.00", "50.00", "300.0"]);
        assert_eq!(&days[2][1..], ["2", "2", "3", "1.50", "50.00", "30.0"]);
        let mut zip=zip_body(request(&app,&format!("/sites/1/export?from=2026-10-27&to=2026-10-28&interval=week&compare=off{filter}"),&cookie,None).await).await;
        let weeks = csv_rows(&mut zip, "visitors.csv");
        assert_eq!(weeks.len(), 2);
        assert_eq!(weeks[1][0], "2026-10-27");
        assert_eq!(&weeks[1][1..], ["2", "2", "3", "1.50", "50.00", "30.0"]);
    }
}

#[tokio::test]
async fn hourly_exports_disambiguate_dst_and_keep_session_metrics_in_the_final_bucket() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','America/New_York',0)", [])?;
        // One visit crosses the repeated 01:00 hour on the autumn clock change.
        for instant in ["2025-11-02T05:50:00Z", "2025-11-02T06:10:00Z"] {
            let stamp = chrono::DateTime::parse_from_rfc3339(instant).unwrap().timestamp();
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,'2025-11-02',zeroblob(16),randomblob(16),'/','Google','US','Chrome','Desktop')", [stamp])?;
        }
        Ok(())
    }).await.unwrap();
    let response = request(
        &app,
        "/sites/1/export?from=2025-11-02&to=2025-11-02&interval=hour",
        &cookie,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut archive = zip_body(response).await;
    let rows = csv_rows(&mut archive, "visitors.csv");
    assert_eq!(rows.len(), 26, "25 civil hours plus the header");
    let early = rows
        .iter()
        .find(|row| row[0] == "2025-11-02T01:00:00-04:00")
        .unwrap();
    let late = rows
        .iter()
        .find(|row| row[0] == "2025-11-02T01:00:00-05:00")
        .unwrap();
    assert_eq!(&early[1..4], ["1", "1", "1"]);
    assert_eq!(
        early[6], "0.0",
        "A mixed-metric CSV row has a known zero duration before completion"
    );
    assert_eq!(&late[1..], ["1", "1", "1", "2.00", "0.00", "1200.0"]);
    assert_eq!(
        &csv_rows(&mut archive, "summary.csv")[1][3..6],
        ["1", "1", "2"]
    );
}

#[tokio::test]
async fn export_requires_owner_and_respects_segment_without_table_pagination() {
    let (app, db, cookie, _) = fixture().await;
    export_fixture(&db).await;
    let path = "/sites/1/export?from=2026-10-27&to=2026-10-27&detail=page&search=missing&offset=100&sort=name&order=asc";
    assert_eq!(
        request(&app, path, "", None).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        request(&app, "/sites/2/export", &cookie, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "/sites/1/export?from=nope", &cookie, None)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let response = request(&app, path, &cookie, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/zip");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("owlytics-example.com-2026-10-27-2026-10-27.zip")
    );
    let mut archive = zip_body(response).await;
    assert_eq!(archive.len(), 22);
    for name in [
        "channels.csv",
        "utm_mediums.csv",
        "utm_sources.csv",
        "utm_campaigns.csv",
        "utm_contents.csv",
        "utm_terms.csv",
    ] {
        assert_eq!(
            csv_rows(&mut archive, name).len(),
            1,
            "Legacy visits have no invented acquisition rows"
        );
    }
    let summary = csv_rows(&mut archive, "summary.csv");
    assert_eq!(&summary[1][3..6], &["106", "106", "106"]);
    assert_eq!(csv_rows(&mut archive, "pages.csv").len(), 101);
    assert!(csv_rows(&mut archive, "sources.csv")[1][0].starts_with("'="));
    let filtered = format!("{path}&page=%2Fpage%2F105");
    let mut archive = zip_body(request(&app, &filtered, &cookie, None).await).await;
    let pages = csv_rows(&mut archive, "pages.csv");
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1][0], "/page/105");
    assert_eq!(
        &csv_rows(&mut archive, "summary.csv")[1][3..6],
        &["1", "1", "1"]
    );
}

#[tokio::test]
async fn expanded_reports_have_sort_search_pagination_and_working_menu_links() {
    let (app, db, cookie, _) = fixture().await;
    export_fixture(&db).await;
    let base = "/sites/1?from=2026-10-27&to=2026-10-27&detail=page&sort=name&order=asc";
    let html = body(request(&app, base, &cookie, None).await).await;
    assert!(html.contains("class=\"report-table\""));
    assert_eq!(html.matches("<th scope=\"row\">").count(), 50);
    assert!(html.contains("1–50 of 106"));
    assert!(html.contains("aria-sort=\"ascending\""));
    assert!(html.contains("Graph interval"));
    assert!(html.contains("/sites/1/export?"));
    assert!(!html.contains("row-detail"));
    let html = body(request(&app, &format!("{base}&offset=100"), &cookie, None).await).await;
    assert_eq!(html.matches("<th scope=\"row\">").count(), 6);
    assert!(html.contains("101–106 of 106"));
    let html = body(request(&app, &format!("{base}&search=PAGE%2F105"), &cookie, None).await).await;
    assert_eq!(html.matches("<th scope=\"row\">").count(), 1);
    assert!(html.contains("1–1 of 1"));
    for query in [
        "sort=unsafe",
        "order=random",
        "offset=1000001",
        "detail=unknown",
    ] {
        assert_eq!(
            request(&app, &format!("/sites/1?{query}"), &cookie, None)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn unavailable_imported_metrics_stay_blank_in_exports_and_tables() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
        c.execute(
            "INSERT INTO import_batches VALUES(1,1,'2026-10-01','2026-10-01',0)",
            [],
        )?;
        c.execute("INSERT INTO import_daily VALUES(1,'2026-10-01',1,5,12)", [])?;
        c.execute(
            "INSERT INTO import_breakdowns VALUES(1,'2026-10-01','page','/old',1,12)",
            [],
        )?;
        c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1790899200,'2026-10-02',zeroblob(16),zeroblob(16),'/native','','','Safari','Mobile')", [])?;
        Ok(())
    })
    .await
    .unwrap();
    let html = body(
        request(
            &app,
            "/sites/1?from=2026-10-01&to=2026-10-01&detail=page",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(html.contains("Unavailable in this historical export"));
    let mut archive = zip_body(
        request(
            &app,
            "/sites/1/export?from=2026-10-01&to=2026-10-01",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    let summary = csv_rows(&mut archive, "summary.csv");
    assert_eq!(&summary[1][3..], &["5", "", "12", "", "", ""]);
    let pages = csv_rows(&mut archive, "pages.csv");
    assert_eq!(pages[1], vec!["/old", "", "12", "", "", ""]);
    let mut archive = zip_body(
        request(
            &app,
            "/sites/1/export?from=2026-10-01&to=2026-10-02",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    let pages = csv_rows(&mut archive, "pages.csv");
    assert_eq!(
        pages.iter().find(|row| row[0] == "/native").unwrap()[1],
        "1"
    );
    // Unknown history must not turn into a zero-valued comparison line,
    // including a week combining unknown and measured historical days.
    db.call(|c| c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1790985600,'2026-10-03',randomblob(16),randomblob(16),'/native','','','Safari','Mobile')", [])).await.unwrap();
    for metric in ["visits", "bounce_rate", "visit_duration"] {
        for interval in ["day", "week"] {
            let html = body(request(&app, &format!("/sites/1?from=2026-10-03&to=2026-10-04&metric={metric}&interval={interval}&compare=previous&compare_match=date"), &cookie, None).await).await;
            assert!(html.contains("Previous: —"), "{metric}/{interval}");
            let first = html
                .split("class=\"chart-hit\"")
                .nth(1)
                .unwrap()
                .split('>')
                .next()
                .unwrap();
            assert!(first.contains("data-previous-value=\"—\""));
            assert!(
                first.contains("data-previous-y=\"\""),
                "Missing history has no marker"
            );
            assert!(
                first.contains("data-change=\"\""),
                "Missing history has no percentage change"
            );
            if interval == "week" {
                assert!(!html.contains("class=\"chart-previous\""));
            }
        }
    }
}

#[tokio::test]
async fn known_empty_comparisons_differ_from_missing_history_and_measured_zero_rates() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0);
        INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES
        (1,1738497600,'2025-02-02',x'00000000000000000000000000000001',randomblob(16),'/','Google','US','Chrome','Desktop'),
        (1,1738497720,'2025-02-02',x'00000000000000000000000000000001',randomblob(16),'/done','Google','US','Chrome','Desktop'),
        (1,1738497600,'2025-02-02',x'00000000000000000000000000000002',randomblob(16),'/','Google','US','Chrome','Desktop');")).await.unwrap();
    for (current, previous, relative, bounce) in [
        ("2025-02-02", "2025-02-01", "↗ 100%", None),
        ("2025-02-01", "2025-01-31", "→ 0%", None),
        ("2025-02-01", "2025-02-02", "↘ 100%", Some("↘ 50%")),
    ] {
        for (metric, label) in [
            ("visitors", "Unique visitors"),
            ("visits", "Total visits"),
            ("pageviews", "Total pageviews"),
            ("views_per_visit", "Views per visit"),
            ("bounce_rate", "Bounce rate"),
            ("visit_duration", "Visit duration"),
        ] {
            let html = body(request(&app, &format!("/sites/1?from={current}&to={current}&compare=custom&compare_from={previous}&compare_to={previous}&interval=day&metric={metric}"), &cookie, None).await).await;
            let card = html
                .split(&format!("<span>{label}</span>"))
                .nth(1)
                .unwrap()
                .split("</a>")
                .next()
                .unwrap();
            let expected = if metric == "bounce_rate" {
                bounce
            } else {
                Some(relative)
            };
            if let Some(change) = expected {
                assert!(
                    card.contains(change),
                    "{metric} {current} vs {previous}: {card}"
                );
            } else {
                assert!(
                    !card.contains("metric-change"),
                    "A rate needs previous visits: {card}"
                );
            }
            let point = html
                .split("class=\"chart-hit\"")
                .nth(1)
                .unwrap()
                .split('>')
                .next()
                .unwrap();
            let chart_expected = if metric == "visit_duration" {
                None
            } else {
                expected
            };
            let change = chart_expected
                .unwrap_or("")
                .replace('%', if metric == "bounce_rate" { " pp" } else { "%" });
            assert!(
                point.contains(&format!("data-change=\"{change}\"")),
                "{metric} {current} vs {previous}: {point}"
            );
        }
    }
}

#[tokio::test]
async fn bounce_rate_comparisons_show_percentage_points_including_a_measured_zero() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0);
        INSERT INTO import_batches VALUES(1,1,'2026-10-25','2026-10-26',0);
        INSERT INTO import_daily VALUES(1,'2026-10-25',1,5,10),(1,'2026-10-26',1,5,10);
        INSERT INTO import_metrics VALUES(1,'2026-10-25','total','',1,5,5,2,100,NULL),(1,'2026-10-26','total','',1,5,5,1,100,NULL);")).await.unwrap();
    for expected in ["↘ 20%", "↗ 20%"] {
        let html = body(
            request(
                &app,
                "/sites/1?from=2026-10-26&to=2026-10-26&metric=bounce_rate&compare=previous&compare_match=date",
                &cookie,
                None,
            )
            .await,
        )
        .await;
        let card = html
            .split("<span>Bounce rate</span>")
            .nth(1)
            .unwrap()
            .split("</a>")
            .next()
            .unwrap();
        assert!(card.contains("Change in percentage points"));
        assert!(card.contains(expected), "{card}");
        assert!(html.contains(&format!("data-change=\"{}\"", expected.replace('%', " pp"))));
        assert!(
            html.contains("width=\"1000.00\" height=\"300\""),
            "A single point covers the whole plot"
        );
        assert!(html.contains("data-x=\"500.00\""));
        db.call(|c| {
            c.execute(
                "UPDATE import_metrics SET bounces=0 WHERE day='2026-10-25'",
                [],
            )
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn isolated_comparison_values_remain_visible_between_unknown_imports() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0);
        INSERT INTO import_batches VALUES(1,1,'2025-02-01','2025-02-06',0);
        INSERT INTO import_daily VALUES(1,'2025-02-01',1,5,10),(1,'2025-02-02',1,5,10),(1,'2025-02-03',1,5,10),(1,'2025-02-04',1,5,10),(1,'2025-02-05',1,5,10),(1,'2025-02-06',1,5,10);
        INSERT INTO import_metrics VALUES(1,'2025-02-02','total','',1,7,7,2,100,NULL),(1,'2025-02-04','total','',1,5,5,1,100,NULL),(1,'2025-02-05','total','',1,5,5,1,100,NULL),(1,'2025-02-06','total','',1,5,5,1,100,NULL);")).await.unwrap();
    let html = body(request(&app, "/sites/1?from=2025-02-04&to=2025-02-06&metric=visits&interval=day&compare=previous&compare_match=date", &cookie, None).await).await;
    assert!(html.contains("<polyline class=\"chart-previous\" points=\"500.00,52.50\"/>"));
    assert!(
        html.contains("<path class=\"chart-point chart-point-previous\" d=\"M500.00 52.50h0\"/>"),
        "A one-point polyline has no visible stroke; an isolated known comparison needs a dot"
    );
    assert_eq!(
        html.matches("data-previous-value=\"—\"").count(),
        2,
        "Missing import metrics remain unknown, not a line to zero"
    );
}

#[tokio::test]
async fn comparison_chart_keeps_unequal_dst_series_on_one_axis() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','America/New_York',0)", [])?;
        // Both visits are at 23:10 local, on a 25-hour day and then a 24-hour day.
        for (instant, day) in [("2025-11-03T04:10:00Z", "2025-11-02"), ("2025-11-04T04:10:00Z", "2025-11-03")] {
            let stamp = chrono::DateTime::parse_from_rfc3339(instant).unwrap().timestamp();
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,?2,randomblob(16),randomblob(16),'/','','US','Chrome','Desktop')", rusqlite::params![stamp,day])?;
        }
        Ok(())
    }).await.unwrap();
    let html = body(
        request(
            &app,
            "/sites/1?from=2025-11-03&to=2025-11-03&interval=hour&compare=previous&compare_match=date",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    let points: Vec<_> = html
        .split("class=\"chart-hit\"")
        .skip(1)
        .map(|part| part.split('>').next().unwrap())
        .collect();
    assert_eq!(points.len(), 25);
    assert!(
        html.contains(
            "<div class=\"chart-scale\" aria-hidden=\"true\"><span>1</span><span>0</span></div>"
        ),
        "Small counts have distinct integer axis labels"
    );
    assert!(points[23].contains("data-date=\"Mon, 03 Nov 2025, 23:00 EST\""));
    assert!(points[23].contains("data-x=\"958.33\" data-y=\"20.00\""));
    assert!(points[24].contains("data-date=\"\" data-value=\"—\""));
    assert!(
        points[24].contains(
            "data-previous-date=\"Sun, 02 Nov 2025, 23:00 EST\" data-previous-value=\"1\""
        )
    );
    assert!(points[24].contains("data-x=\"1000.00\" data-y=\"\" data-previous-y=\"20.00\""));
    assert!(points[24].contains("data-change=\"\""));
    let line = html
        .split("<polyline class=\"chart-line\" points=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert!(line.ends_with("958.33,20.00"));
}

#[tokio::test]
async fn filtered_bounce_rates_stay_weighted_in_charts_comparisons_and_csv() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
        // Monday: one bounce from one entrance, plus an internal visit.
        // Tuesday: two entrances and no bounces. Together: 1/3, not 1/4
        // matching visits, nor the 50% average of the daily percentages.
        for (id, visitor, day, offset, path) in [
            (1, 1, 26, 0, "/"),
            (2, 1, 26, 30, "/pricing"),
            (3, 2, 26, 60, "/pricing"),
            (4, 3, 27, 0, "/pricing"),
            (5, 3, 27, 30, "/docs"),
            (6, 4, 27, 60, "/pricing"),
            (7, 4, 27, 90, "/docs"),
        ] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,?2,?3,?4,?5,'','','Chrome','Desktop')",
                rusqlite::params![1792972800_i64+(day-26)*86400+offset,format!("2026-10-{day}"), i128::to_be_bytes(visitor).as_slice(),i128::to_be_bytes(id).as_slice(),path])?;
        }
        Ok(())
    }).await.unwrap();
    let base = "/sites/1?from=2026-10-26&to=2026-10-27&page=%2Fpricing&metric=bounce_rate";
    for interval in ["day", "week", "month"] {
        let html =
            body(request(&app, &format!("{base}&interval={interval}"), &cookie, None).await).await;
        assert!(html.contains("<span>Bounce rate</span><strong>33%</strong>"));
        if interval == "day" {
            assert!(html.contains("Mon, 26 Oct 2026: 100% bounce rate"));
            assert!(html.contains("Tue, 27 Oct 2026: 0% bounce rate"));
        } else {
            assert!(html.contains("33% bounce rate"));
            assert!(
                html.contains(if interval == "week" {
                    "data-date=\"Partial week of 26 Oct\""
                } else {
                    "data-date=\"Partial of October 2026\""
                }),
                "Grouped labels show the actual selected dates"
            );
        }
    }
    let html = body(request(&app, "/sites/1?from=2026-10-27&to=2026-10-27&page=%2Fpricing&metric=bounce_rate&interval=day&compare=previous&compare_match=date", &cookie, None).await).await;
    assert!(html.contains("Previous: 100%"));
    let mut archive = zip_body(
        request(
            &app,
            "/sites/1/export?from=2026-10-26&to=2026-10-27&page=%2Fpricing",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    let summary = csv_rows(&mut archive, "summary.csv");
    assert_eq!(&summary[1][3..6], &["4", "4", "4"]);
    assert_eq!(summary[1][7], "33.33");
    let daily = csv_rows(&mut archive, "visitors.csv");
    assert_eq!(
        (daily[1][5].as_str(), daily[2][5].as_str()),
        ("100.00", "0.00")
    );
    assert_eq!(csv_rows(&mut archive, "pages.csv")[1][3], "33.33");
}

#[tokio::test]
async fn page_filters_switch_engagement_metrics_charts_and_exports() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)",[])?;
        c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1760918400,'2025-10-20',zeroblob(16),zeroblob(16),'/docs','','','Chrome','Desktop')",[])?;
        Ok(())
    }).await.unwrap();
    owlytics::engagement::collect(
        &db,
        owlytics::engagement::Event {
            domain: "example.com".into(),
            id: "00".repeat(16),
            time_ms: 30000,
            scroll_depth: 75,
        },
        "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36".into(),
        || 1760918430,
    )
    .await
    .unwrap();
    let path = "/sites/1?from=2025-10-20&to=2025-10-20&page=%2Fdocs&metric=visit_duration";
    let html = body(request(&app, path, &cookie, None).await).await;
    assert!(html.contains("<span>Time on page</span><strong>30s</strong>"));
    assert!(html.contains("<span>Scroll depth</span><strong>75%</strong>"));
    assert!(!html.contains("<span>Visit duration</span>"));
    assert!(!html.contains("<span>Views per visit</span>"));
    assert!(html.contains("aria-label=\"Time on page from"));
    assert!(html.contains("30s time on page"));
    let html = body(
        request(
            &app,
            "/sites/1?from=2025-10-20&to=2025-10-20&metric=time_on_page",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(html.contains("aria-label=\"Visit duration from"));
    let html = body(
        request(
            &app,
            "/sites/1?from=2025-10-20&to=2025-10-20&detail=page&sort=scroll_depth",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(html.contains("Scroll depth"));
    assert!(html.contains("Time on page"));
    assert!(html.contains("<td>75%</td>"));
    assert!(html.contains("<td>30s</td>"));
    let mut archive = zip_body(
        request(
            &app,
            "/sites/1/export?from=2025-10-20&to=2025-10-20&page=%2Fdocs",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    let summary = csv_rows(&mut archive, "summary.csv");
    assert_eq!(
        &summary[0][6..],
        &[
            "scroll_depth_percent",
            "bounce_rate_percent",
            "time_on_page_seconds"
        ]
    );
    assert_eq!(&summary[1][6..], &["75.00", "100.00", "30.0"]);
    let pages = csv_rows(&mut archive, "pages.csv");
    assert_eq!(&pages[0][4..], &["time_on_page", "scroll_depth"]);
    assert_eq!(&pages[1][4..], &["30.00", "75.00"]);
    db.call(|c|c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1761091200,'2025-10-22',randomblob(16),x'01010101010101010101010101010101','/docs','','','Chrome','Desktop')",[])).await.unwrap();
    owlytics::engagement::collect(
        &db,
        owlytics::engagement::Event {
            domain: "example.com".into(),
            id: "01".repeat(16),
            time_ms: 10000,
            scroll_depth: 50,
        },
        "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36".into(),
        || 1761091210,
    )
    .await
    .unwrap();
    let html = body(
        request(
            &app,
            "/sites/1?from=2025-10-20&to=2025-10-22&page=%2Fdocs&metric=time_on_page&interval=day",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(html.matches("class=\"chart-line\"").count(), 2);
    assert!(html.contains("Tue, 21 Oct 2025: — time on page"));
}

#[tokio::test]
async fn acquisition_tabs_filters_drilldowns_and_exports_share_the_captured_data() {
    use owlytics::{
        acquisition::Campaign,
        analytics::{self, Event},
    };
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    let tags = |source: &str, medium: &str, name: &str| {
        Some(Campaign {
            source: Some(source.into()),
            medium: Some(medium.into()),
            name: Some(name.into()),
            content: Some("=1+1".into()),
            ..Default::default()
        })
    };
    let malicious = "<Launch & \"quote\">";
    for (id, second, ip, campaign) in [
        (1, 0, "192.0.2.1", tags("FB-Ads", "CPC", malicious)),
        (2, 1900, "192.0.2.1", tags("fb-ads", "cpc", "B")),
        (3, 2000, "192.0.2.2", tags("Google", "cpc", "B")),
        (4, 2100, "192.0.2.3", None),
        (5, 2200, "192.0.2.4", Some(Campaign::default())),
    ] {
        analytics::collect(
            &db,
            Event {
                domain: "example.com".into(),
                path: format!("/{id}"),
                referrer: "https://google.com/search?private=x".into(),
                id: format!("{id:032x}"),
                campaign,
            },
            ip.parse().unwrap(),
            "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36".into(),
            move || 1793059200 + second,
        )
        .await
        .unwrap();
    }
    let base = "/sites/1?from=2026-10-27&to=2026-10-27&compare=off&interval=day";
    let html = body(
        request(
            &app,
            &format!("{base}&sources_tab=utm_campaign"),
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(html.contains("class=\"dropdown campaign-menu\""));
    assert!(html.contains("<summary aria-current=\"page\">UTM campaigns"));
    assert!(html.contains("Some visits have no campaign data."));
    assert!(html.contains("&lt;Launch &amp; &quot;quote&quot;&gt;"));
    assert!(!html.contains(malicious));
    assert!(html.contains("class=\"row-percentage\">50.0%"));
    assert!(html.contains("class=\"row-percentage\">100.0%"));
    let selected = report_link(&html, "&lt;Launch &amp;");
    let selected_html = body(request(&app, &selected, &cookie, None).await).await;
    assert!(selected_html.contains("Remove utm_campaign filter"));
    assert!(selected_html.contains("<span>Total pageviews</span><strong>1</strong>"));
    let expanded = report_link(&selected_html, "aria-label=\"Expand report\"");
    let expanded_html = body(request(&app, &expanded, &cookie, None).await).await;
    assert!(expanded_html.contains("panel-utm_campaign expanded-report"));
    assert!(expanded_html.contains("Bounce rate"));
    for (dimension, _) in owlytics::acquisition::DIMENSIONS {
        let response = request(&app, &format!("{base}&detail={dimension}"), &cookie, None).await;
        assert_eq!(response.status(), StatusCode::OK, "{dimension}");
        assert!(
            body(response)
                .await
                .contains(&format!("panel-{dimension} expanded-report")),
            "{dimension}"
        );
    }
    let removed = report_link(&selected_html, "Remove utm_campaign filter");
    let removed_html = body(request(&app, &removed, &cookie, None).await).await;
    assert!(removed_html.contains("<span>Total pageviews</span><strong>5</strong>"));
    assert!(removed.contains("sources_tab=utm_campaign"));
    let channels =
        body(request(&app, &format!("{base}&sources_tab=channel"), &cookie, None).await).await;
    let social = report_link(&channels, "Paid Social");
    let social_html = body(request(&app, &social, &cookie, None).await).await;
    assert!(social_html.contains("panel-source"));
    assert!(social_html.contains("Remove channel filter"));
    assert!(social_html.contains("<span>Total pageviews</span><strong>2</strong>"));
    let facebook = report_link(&social_html, "Facebook ·");
    let facebook_html = body(request(&app, &facebook, &cookie, None).await).await;
    assert!(facebook_html.contains("panel-referrer"));
    assert!(facebook_html.contains("google.com/search"));
    assert!(facebook_html.contains("Remove channel filter"));
    assert!(facebook_html.contains("Remove source filter"));
    let clear = report_link(&facebook_html, "Clear filters");
    assert!(clear.contains("sources_tab=source"));
    let options =
        body(request(&app, &format!("{base}&filter=utm_medium"), &cookie, None).await).await;
    assert!(options.contains("value=\"CPC\""));
    assert!(options.contains("value=\"cpc\""));
    let mut archive = zip_body(
        request(
            &app,
            &format!("/sites/1/export?from=2026-10-27&to=2026-10-27"),
            &cookie,
            None,
        )
        .await,
    )
    .await;
    let campaigns = csv_rows(&mut archive, "utm_campaigns.csv");
    assert_eq!(campaigns.len(), 3);
    assert!(
        campaigns
            .iter()
            .any(|row| row[0] == malicious && row[1] == "1")
    );
    assert_eq!(csv_rows(&mut archive, "utm_contents.csv")[1][0], "'=1+1");
    let selected_export = report_link(&selected_html, "Export stats");
    let mut archive = zip_body(request(&app, &selected_export, &cookie, None).await).await;
    assert_eq!(csv_rows(&mut archive, "utm_campaigns.csv").len(), 2);
    assert_eq!(csv_rows(&mut archive, "summary.csv")[1][5], "1");
}

#[tokio::test]
async fn import_switch_preserves_audience_links_forms_exports_and_comparison_intervals() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c|c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0),(2,'empty.example','UTC',0);
        INSERT INTO import_batches VALUES(1,1,'2025-01-01','2025-01-01',0);
        INSERT INTO import_daily VALUES(1,'2025-01-01',1,10,20);
        INSERT INTO import_breakdowns VALUES(1,'2025-01-01','browser','Chrome',1,20);
        INSERT INTO import_metrics(site_id,day,dimension,value,batch_id,visitors,visits,bounces,duration) VALUES(1,'2025-01-01','browser','Chrome',1,10,10,2,20);
        INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1738368000,'2025-02-01',zeroblob(16),zeroblob(16),'/','','US','Chrome','Desktop');")).await.unwrap();
    let base = "/sites/1?from=2025-02-01&to=2025-02-01&browser=Chrome&compare=custom&compare_from=2025-01-01&compare_to=2025-01-01&interval=hour";
    let included = body(request(&app, base, &cookie, None).await).await;
    assert!(included.contains("role=\"switch\" aria-checked=\"true\""));
    assert_eq!(
        included.matches("class=\"chart-hit\"").count(),
        1,
        "Compatible filtered history requires daily buckets on both sides"
    );
    assert!(included.contains("Previous: 10"));
    let path = format!("{base}&with_imported=false");
    let html = body(request(&app, &path, &cookie, None).await).await;
    assert!(
        html.contains(
            "role=\"switch\" aria-checked=\"false\" name=\"with_imported\" value=\"true\""
        )
    );
    assert_eq!(
        html.matches("class=\"chart-hit\"").count(),
        24,
        "Hiding history permits hourly charts on both sides"
    );
    assert!(!html.contains("Historical data is excluded"));
    assert!(!html.contains("Some historical"));
    assert!(
        html.contains("name=\"with_imported\" value=\"false\""),
        "All other forms retain the visibility choice"
    );
    for marker in ["Show pageviews", "class=\"clear-filters\"", "Export stats"] {
        let url = report_link(&html, marker);
        assert!(url.contains("with_imported=false"), "{marker}");
    }
    let mut zip =
        zip_body(request(&app, &report_link(&html, "Export stats"), &cookie, None).await).await;
    assert_eq!(&csv_rows(&mut zip, "summary.csv")[1][3], "1");
    let mut note = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("readme.txt").unwrap(), &mut note).unwrap();
    assert!(note.contains("Imported history switched off: true"));
    assert!(note.contains("Imported history excluded by filters or rolling window: false"));
    assert!(!note.contains("Historical data partly or wholly excluded from:"));
    let html = body(request(&app, "/sites/2", &cookie, None).await).await;
    assert!(!html.contains("Include imported data"));
    for route in ["/sites/1", "/sites/1/export", "/sites/1/filter"] {
        assert_eq!(
            request(
                &app,
                &format!("{route}?with_imported=invalid"),
                &cookie,
                None
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn comparison_defaults_are_off_and_enabled_comparisons_match_weekdays() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    for suffix in [
        "",
        "?compare=off",
        "?period=all&compare=previous",
        "?period=realtime&compare=year",
    ] {
        let html = body(request(&app, &format!("/sites/1{suffix}"), &cookie, None).await).await;
        assert!(!html.contains("class=\"dropdown comparison-picker\""));
    }
    let html = body(
        request(
            &app,
            "/sites/1?from=2025-11-03&to=2025-11-03&compare=previous",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(html.contains("title=\"Oct 27, 2025\"><svg"));
    assert!(report_link(&html, ">Match day of week<span").contains("compare_match=weekday"));
    let html = body(
        request(
            &app,
            "/sites/1?from=2025-11-03&to=2025-11-03&compare=previous&compare_match=date",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(html.contains("title=\"Nov 2, 2025\"><svg"));
}

#[tokio::test]
async fn realtime_cards_reports_and_csv_use_their_respective_windows() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)",[])?;
        let now=auth::now();
        for (index,person,offset,path,source,browser) in [(1,1,1200,"/older","OldOnly","Firefox"),(2,2,90,"/active","Google","Chrome"),(3,3,60,"/recent","GitHub","Safari"),(4,2,90,"/active","Google","Chrome")] {
            let stamp=now-offset;
            let day=chrono::DateTime::from_timestamp(stamp,0).unwrap().date_naive().to_string();
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,?2,?3,?4,?5,?6,'US',?7,'Desktop')",rusqlite::params![stamp,day,vec![person as u8;16],vec![index as u8;16],path,source,browser])?;
        }
        Ok(())
    }).await.unwrap();
    let target = "/sites/1?period=realtime&metric=bounce_rate&compare=year";
    let html = body(request(&app, target, &cookie, None).await).await;
    assert!(html.contains("data-realtime=\"true\""));
    assert!(html.contains("class=\"metric realtime-current\" data-value=\"2\""));
    assert!(html.contains("aria-label=\"United States · 2 current visitors\""));
    assert!(html.contains("<span>Unique visitors (last 30 min)</span><strong>3</strong>"));
    assert!(html.contains("<span>Total pageviews (last 30 min)</span><strong>4</strong>"));
    assert!(!html.contains("<span>Total visits</span>"));
    assert!(!html.contains("class=\"current-visitors\""));
    assert!(!html.contains("class=\"dropdown comparison-picker\""));
    assert!(html.contains("<span>Source</span><span>Current visitors</span>"));
    assert!(!html.contains("/older</span>"));
    assert!(!html.contains(">OldOnly</span>"));
    assert_eq!(html.matches("class=\"chart-hit\"").count(), 30);
    assert!(html.contains("data-date=\"30 minutes ago\""));
    assert!(html.contains("data-date=\"1 minute ago\""));
    for label in ["-30m", "-25m", "-20m", "-15m", "-10m", "-5m"] {
        assert!(html.contains(&format!("<span>{label}</span>")));
    }
    let mut zip =
        zip_body(request(&app, &report_link(&html, "Export stats"), &cookie, None).await).await;
    let summary = csv_rows(&mut zip, "summary.csv");
    assert_eq!(
        summary[0],
        vec![
            "site",
            "from",
            "to",
            "current_visitors",
            "visitors",
            "pageviews"
        ]
    );
    assert_eq!(&summary[1][3..], &["2", "3", "4"]);
    let pages = csv_rows(&mut zip, "pages.csv");
    assert_eq!(pages.len(), 3);
    assert!(pages.iter().skip(1).all(|row| row[0] != "/older"));
    let chart = csv_rows(&mut zip, "visitors.csv");
    assert_eq!(chart[0], vec!["date", "visitors", "pageviews"]);
    assert_eq!(chart.len(), 31);
    let mut note = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("readme.txt").unwrap(), &mut note).unwrap();
    assert!(note.contains("Breakdown tables and their percentages cover the last 5 minutes"));
    let filtered = body(
        request(
            &app,
            "/sites/1?period=realtime&source=OldOnly",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(filtered.contains("class=\"metric realtime-current\" data-value=\"2\""));
    assert!(filtered.contains("<span>Unique visitors (last 30 min)</span><strong>1</strong>"));
    assert!(!filtered.contains("/older</span>"));
    let zero = body(
        request(
            &app,
            "/sites/1?period=realtime&source=Missing",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(zero.contains("<span>Unique visitors (last 30 min)</span><strong>0</strong>"));
    assert_eq!(
        zero.matches("class=\"chart-hit\"").count(),
        30,
        "An empty realtime report still has a live chart"
    );
}

#[tokio::test]
async fn calendar_navigation_and_chart_links_preserve_the_audience_and_exported_dates() {
    let (app, db, cookie, _) = fixture().await;
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'calendar.example','UTC',0),(2,'empty.example','UTC',0)",[])?;
        c.execute("INSERT INTO import_batches VALUES(1,1,'2023-02-17','2023-02-17',0)",[])?;
        c.execute("INSERT INTO import_daily VALUES(1,'2023-02-17',1,5,7)",[])?;
        for date in ["2024-02-29","2024-03-01"] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,unixepoch(?1)+36000,?1,randomblob(16),randomblob(16),'/docs','Google','US','Chrome','Desktop')",[date])?;
        }
        Ok(())
    }).await.unwrap();
    let path = "/sites/1?period=month&date=2024-02-29&source=Google&metric=pageviews&with_imported=false&detail=page&search=docs&sort=name&order=asc&offset=50&compare=previous&compare_match=date";
    let html = body(request(&app, path, &cookie, None).await).await;
    assert!(html.contains("<span>February 2024</span>"));
    assert!(html.contains("<span>Total pageviews</span><strong>1</strong>"));
    let previous = report_link(&html, "aria-label=\"Previous month\"");
    let next = report_link(&html, "aria-label=\"Following month\"");
    for (target, date) in [(&previous, "2024-01-01"), (&next, "2024-03-01")] {
        let url = url::Url::parse(&format!("https://stats.example.com{target}")).unwrap();
        let params = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        for (key, value) in [
            ("date", date),
            ("period", "month"),
            ("source", "Google"),
            ("metric", "pageviews"),
            ("with_imported", "false"),
            ("compare", "previous"),
            ("compare_match", "date"),
            ("detail", "page"),
            ("search", "docs"),
            ("sort", "name"),
            ("order", "asc"),
        ] {
            assert_eq!(params.get(key).map(|v| v.as_ref()), Some(value), "{key}");
        }
        assert!(!params.contains_key("offset"));
    }
    let next_html = body(request(&app, &next, &cookie, None).await).await;
    assert!(next_html.contains("<span>March 2024</span>"));
    assert!(next_html.contains("<span>Total pageviews</span><strong>1</strong>"));
    assert!(next_html.contains(">/docs<"));
    let mut zip = zip_body(
        request(
            &app,
            &report_link(&next_html, "Export stats"),
            &cookie,
            None,
        )
        .await,
    )
    .await;
    let summary = csv_rows(&mut zip, "summary.csv");
    assert_eq!(
        (&summary[1][1], &summary[1][2]),
        (&"2024-03-01".to_owned(), &"2024-03-31".to_owned())
    );
    let graph = body(
        request(
            &app,
            "/sites/1?from=2024-02-01&to=2024-03-31&interval=month&source=Google&compare=off",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    let zoom = report_link(&graph, "class=\"chart-hit\"");
    assert!(
        zoom.contains("period=month")
            && zoom.contains("date=2024-02-01")
            && zoom.contains("source=Google")
    );
    assert!(!zoom.contains("interval="));
    let monthly = body(request(&app, &zoom, &cookie, None).await).await;
    let day = report_link(&monthly, "class=\"chart-hit\"");
    assert!(day.contains("period=day") && day.contains("date=2024-02-01"));
    let earliest = body(
        request(
            &app,
            "/sites/1?period=month&date=2023-02-01&source=Missing&with_imported=false",
            &cookie,
            None,
        )
        .await,
    )
    .await;
    assert!(
        earliest.contains("aria-disabled=\"true\" aria-label=\"Previous month\""),
        "Navigation bounds use all history, including disabled imports and other audiences"
    );
    let empty = body(request(&app, "/sites/2?period=day", &cookie, None).await).await;
    assert_eq!(empty.matches("aria-disabled=\"true\"").count(), 2);
    let today = body(request(&app, "/sites/1?period=day", &cookie, None).await).await;
    assert!(today.contains("aria-disabled=\"true\" aria-label=\"Following day\""));
    let invalid = request(&app, "/sites/1?period=month&date=2023-02-29", &cookie, None).await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}
