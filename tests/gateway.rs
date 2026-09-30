//! End-to-end behaviour of the gateway over stub PDS nodes.

mod support;

use serde_json::json;
use support::{Harness, Hosted, harness, start_node, stub_did, token};

fn hosted(handle: &str, email: &str) -> Hosted {
    Hosted {
        did: stub_did(handle),
        handle: handle.to_owned(),
        email: email.to_owned(),
        password: "correct-horse".to_owned(),
    }
}

async fn fleet() -> Harness {
    let primary = start_node(
        "primary",
        vec![hosted("alice.rocksky.social", "alice@example.com")],
    )
    .await;
    let radxa = start_node(
        "radxa",
        vec![hosted("bob.rocksky.social", "bob@example.com")],
    )
    .await;
    harness(vec![primary, radxa], |_| {}).await
}

#[tokio::test]
async fn resolves_a_handle_for_the_whole_namespace() {
    let h = fleet().await;

    // A handle hosted on the second node: the gateway must find it by asking
    // the nodes, which is what makes it authoritative for the wildcard.
    let (status, _, body) = h
        .get("/xrpc/com.atproto.identity.resolveHandle?handle=bob.rocksky.social")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["did"], json!(stub_did("bob.rocksky.social")));

    let (status, _, body) = h
        .get("/xrpc/com.atproto.identity.resolveHandle?handle=alice.rocksky.social")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["did"], json!(stub_did("alice.rocksky.social")));
}

#[tokio::test]
async fn an_unclaimed_handle_is_reported_unresolvable() {
    let h = fleet().await;
    let (status, _, body) = h
        .get("/xrpc/com.atproto.identity.resolveHandle?handle=nobody.rocksky.social")
        .await;

    assert_eq!(status, 400);
    assert_eq!(body["error"], json!("UnableToResolveHandle"));
}

#[tokio::test]
async fn rejects_a_malformed_handle() {
    let h = fleet().await;
    let (status, _, body) = h
        .get("/xrpc/com.atproto.identity.resolveHandle?handle=not-a-handle")
        .await;

    assert_eq!(status, 400);
    assert_eq!(body["error"], json!("InvalidRequest"));
}

#[tokio::test]
async fn serves_the_well_known_document_for_a_handle() {
    let h = fleet().await;

    let (status, headers, body) = h
        .get_with(
            "/.well-known/atproto-did",
            &[("host", "bob.rocksky.social")],
        )
        .await;

    assert_eq!(status, 200);
    assert_eq!(
        String::from_utf8_lossy(&body),
        stub_did("bob.rocksky.social")
    );
    assert_eq!(
        headers.get("cache-control").unwrap().to_str().unwrap(),
        "no-store"
    );

    // A handle nobody holds must 404, not answer with someone else's DID.
    let (status, _, _) = h
        .get_with(
            "/.well-known/atproto-did",
            &[("host", "nobody.rocksky.social")],
        )
        .await;
    assert_eq!(status, 404);

    // A domain outside the namespace is not ours to answer for.
    let (status, _, _) = h
        .get_with("/.well-known/atproto-did", &[("host", "alice.bsky.social")])
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn approves_tls_only_for_names_that_exist() {
    let h = fleet().await;

    let (status, _, _) = h
        .get_with("/tls-check?domain=bob.rocksky.social", &[])
        .await;
    assert_eq!(status, 200, "an existing handle needs a certificate");

    let (status, _, _) = h
        .get_with("/tls-check?domain=nobody.rocksky.social", &[])
        .await;
    assert_eq!(status, 404, "an unclaimed name must not be issued a cert");

    let (status, _, _) = h.get_with("/tls-check?domain=evil.example.com", &[]).await;
    assert_eq!(status, 404);

    let (status, _, _) = h.get_with("/tls-check", &[]).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn routes_a_read_to_the_node_hosting_the_repo() {
    let h = fleet().await;

    let (status, headers, body) = h
        .get("/xrpc/com.atproto.repo.getRecord?repo=bob.rocksky.social&collection=app.bsky.feed.post&rkey=1")
        .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["servedBy"], json!("radxa"));
    assert_eq!(Harness::node_header(&headers).as_deref(), Some("radxa"));

    let (_, headers, body) = h
        .get("/xrpc/com.atproto.repo.getRecord?repo=alice.rocksky.social&collection=c&rkey=1")
        .await;
    assert_eq!(body["servedBy"], json!("primary"));
    assert_eq!(Harness::node_header(&headers).as_deref(), Some("primary"));
}

#[tokio::test]
async fn routes_by_the_did_in_a_bearer_token() {
    let h = fleet().await;
    let bob = stub_did("bob.rocksky.social");

    // Warm the registry so the DID is known.
    let _ = h
        .get("/xrpc/com.atproto.identity.resolveHandle?handle=bob.rocksky.social")
        .await;

    let auth = format!("Bearer {}", token(&bob, None));
    let (status, headers, body) = h
        .get_with(
            "/xrpc/com.atproto.server.getSession",
            &[("authorization", &auth)],
        )
        .await;

    assert_eq!(status, 200, "{:?}", String::from_utf8_lossy(&body));
    assert_eq!(Harness::node_header(&headers).as_deref(), Some("radxa"));
}

#[tokio::test]
async fn falls_back_to_the_token_audience_for_an_unknown_did() {
    let h = fleet().await;

    // A DID the gateway has never seen, but whose token names its issuing node.
    let auth = format!(
        "Bearer {}",
        token(
            "did:plc:strangerstrangerstranger",
            Some("did:web:radxa.rocksky.social")
        )
    );
    let (status, headers, _) = h
        .get_with(
            "/xrpc/com.atproto.server.getSession",
            &[("authorization", &auth)],
        )
        .await;

    assert_eq!(status, 200);
    assert_eq!(Harness::node_header(&headers).as_deref(), Some("radxa"));
}

#[tokio::test]
async fn an_explicit_node_parameter_wins() {
    let h = fleet().await;

    // bob lives on radxa, but the operator asked for primary.
    let (_, headers, body) = h
        .get("/xrpc/com.atproto.repo.getRecord?repo=bob.rocksky.social&collection=c&rkey=1&node=primary")
        .await;

    assert_eq!(body["servedBy"], json!("primary"));
    assert_eq!(Harness::node_header(&headers).as_deref(), Some("primary"));
}

#[tokio::test]
async fn creates_an_account_and_remembers_where_it_went() {
    let h = fleet().await;

    let (status, _, body) = h
        .post(
            "/xrpc/com.atproto.server.createAccount",
            json!({
                "handle": "carol.rocksky.social",
                "email": "carol@example.com",
                "password": "correct-horse",
            }),
        )
        .await;

    assert_eq!(status, 200, "{body}");
    let did = body["did"].as_str().unwrap().to_owned();
    assert_eq!(body["handle"], json!("carol.rocksky.social"));

    // Exactly one node received it.
    let created: usize = h.nodes.values().map(|n| n.created().len()).sum();
    assert_eq!(created, 1, "the account must be placed exactly once");

    // The gateway recorded it, so a later read routes without asking anyone.
    let account = h.state.store.account_by_did(&did).await.unwrap().unwrap();
    assert_eq!(account.handle, "carol.rocksky.social");
    assert!(h.nodes.contains_key(&account.node));

    // And no reservation is left behind.
    assert!(h.state.store.list_reservations().await.unwrap().is_empty());
}

#[tokio::test]
async fn refuses_a_handle_another_node_already_holds() {
    let h = fleet().await;

    // bob is hosted on radxa; the gateway must refuse before placing anything.
    let (status, _, body) = h
        .post(
            "/xrpc/com.atproto.server.createAccount",
            json!({
                "handle": "bob.rocksky.social",
                "email": "impostor@example.com",
                "password": "hunter2",
            }),
        )
        .await;

    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("HandleNotAvailable"));

    let created: usize = h.nodes.values().map(|n| n.created().len()).sum();
    assert_eq!(created, 0, "nothing should have been created upstream");
}

#[tokio::test]
async fn refuses_a_handle_it_just_issued() {
    let h = fleet().await;
    let request = json!({
        "handle": "dave.rocksky.social",
        "email": "dave@example.com",
        "password": "correct-horse",
    });

    let (status, _, _) = h
        .post("/xrpc/com.atproto.server.createAccount", request.clone())
        .await;
    assert_eq!(status, 200);

    // A second attempt by someone else must collide.
    let mut other = request.clone();
    other["email"] = json!("someone-else@example.com");
    let (status, _, body) = h
        .post("/xrpc/com.atproto.server.createAccount", other)
        .await;

    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("HandleNotAvailable"));
}

#[tokio::test]
async fn refuses_a_reserved_handle() {
    let h = fleet().await;

    let (status, _, body) = h
        .post(
            "/xrpc/com.atproto.server.createAccount",
            json!({
                "handle": "admin.rocksky.social",
                "email": "someone@example.com",
                "password": "hunter2",
            }),
        )
        .await;

    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("HandleNotAvailable"));
}

#[tokio::test]
async fn concurrent_signups_for_one_handle_produce_one_account() {
    let h = std::sync::Arc::new(fleet().await);

    let attempts = (0..6).map(|i| {
        let h = h.clone();
        tokio::spawn(async move {
            h.post(
                "/xrpc/com.atproto.server.createAccount",
                json!({
                    "handle": "race.rocksky.social",
                    "email": format!("racer{i}@example.com"),
                    "password": "correct-horse",
                }),
            )
            .await
            .0
        })
    });

    let results = futures::future::join_all(attempts).await;
    let wins = results
        .into_iter()
        .filter(|r| r.as_ref().is_ok_and(|s| s.is_success()))
        .count();

    assert_eq!(wins, 1, "exactly one concurrent signup may win the handle");
    let created: usize = h.nodes.values().map(|n| n.created().len()).sum();
    assert_eq!(created, 1, "the handle must exist on exactly one node");
}

#[tokio::test]
async fn login_by_email_finds_the_right_node_and_learns_it() {
    let h = fleet().await;

    let (status, headers, body) = h
        .post(
            "/xrpc/com.atproto.server.createSession",
            json!({"identifier": "bob@example.com", "password": "correct-horse"}),
        )
        .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(body["servedBy"], json!("radxa"));
    assert_eq!(Harness::node_header(&headers).as_deref(), Some("radxa"));

    // The gateway now knows where that email lives.
    assert_eq!(
        h.state
            .store
            .hint("bob@example.com")
            .await
            .unwrap()
            .as_deref(),
        Some("radxa")
    );
    // And it learned the account itself from the session response.
    let did = stub_did("bob.rocksky.social");
    assert!(h.state.store.account_by_did(&did).await.unwrap().is_some());
}

#[tokio::test]
async fn a_wrong_password_is_answered_not_retried_elsewhere() {
    let h = fleet().await;

    let (status, _, body) = h
        .post(
            "/xrpc/com.atproto.server.createSession",
            json!({"identifier": "bob@example.com", "password": "wrong"}),
        )
        .await;

    assert_eq!(status, 401);
    assert_eq!(
        body["error"],
        json!("InvalidPassword"),
        "the holding node's verdict must be returned verbatim"
    );
}

#[tokio::test]
async fn login_for_an_unknown_account_reports_not_found() {
    let h = fleet().await;

    let (status, _, body) = h
        .post(
            "/xrpc/com.atproto.server.createSession",
            json!({"identifier": "ghost@example.com", "password": "whatever"}),
        )
        .await;

    assert_eq!(status, 401);
    assert_eq!(body["error"], json!("AccountNotFound"));
}

#[tokio::test]
async fn merges_list_repos_across_the_fleet() {
    let h = fleet().await;

    let (status, _, body) = h.get("/xrpc/com.atproto.sync.listRepos").await;
    assert_eq!(status, 200, "{body}");

    let repos = body["repos"].as_array().unwrap();
    assert_eq!(repos.len(), 2, "both nodes' repos should appear");

    let nodes: std::collections::HashSet<&str> =
        repos.iter().filter_map(|r| r["node"].as_str()).collect();
    assert_eq!(nodes.len(), 2);

    // Per-node cursors cannot be merged, so none is returned.
    assert!(
        body.get("cursor").is_none(),
        "a merged cursor would be a lie"
    );
}

#[tokio::test]
async fn a_fanout_survives_one_node_being_down() {
    let h = fleet().await;
    h.nodes["radxa"].set_offline(true);

    let (status, _, body) = h.get("/xrpc/com.atproto.sync.listRepos").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["repos"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn describe_server_advertises_the_handle_domains() {
    let h = fleet().await;

    let (status, _, body) = h.get("/xrpc/com.atproto.server.describeServer").await;
    assert_eq!(status, 200);
    assert_eq!(
        body["availableUserDomains"],
        json!([".rocksky.social"]),
        "clients read this to offer handles"
    );
}

#[tokio::test]
async fn reports_health_and_readiness() {
    let h = fleet().await;

    let (status, _, body) = h.get("/health").await;
    assert_eq!(status, 200);
    assert_eq!(body["nodesTotal"], json!(2));

    let (status, _, body) = h.get("/health/ready").await;
    assert_eq!(status, 200);
    assert_eq!(body["ready"], json!(true));

    let (status, _, body) = h.get("/xrpc/_health").await;
    assert_eq!(status, 200);
    assert!(body["version"].is_string());
}

#[tokio::test]
async fn the_admin_api_requires_its_token() {
    let h = fleet().await;

    let (status, _, _) = h.get_with("/admin/nodes", &[]).await;
    assert_eq!(status, 401);

    let (status, _, _) = h
        .get_with("/admin/nodes", &[("authorization", "Bearer wrong")])
        .await;
    assert_eq!(status, 401);

    let (status, _, body) = h
        .get_with(
            "/admin/nodes",
            &[("authorization", "Bearer test-admin-token")],
        )
        .await;
    assert_eq!(status, 200);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["nodes"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn admin_resolve_explains_a_routing_decision() {
    let h = fleet().await;

    let (status, _, body) = h
        .get_with(
            "/admin/resolve?subject=bob.rocksky.social&live=true",
            &[("authorization", "Bearer test-admin-token")],
        )
        .await;

    assert_eq!(status, 200);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["kind"], json!("handle"));
    assert_eq!(parsed["ownedNamespace"], json!(true));
    assert_eq!(parsed["node"], json!("radxa"));
    assert_eq!(parsed["did"], json!(stub_did("bob.rocksky.social")));
}

#[tokio::test]
async fn metrics_expose_the_fleet() {
    let h = fleet().await;
    let _ = h.get("/xrpc/com.atproto.sync.listRepos").await;

    let (status, _, body) = h.get_with("/metrics", &[]).await;
    assert_eq!(status, 200);

    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("pdsgw_requests_total"));
    assert!(text.contains("pdsgw_node_up{node=\"primary\"}"));
    assert!(text.contains("pdsgw_node_up{node=\"radxa\"}"));
}

#[tokio::test]
async fn signups_can_be_closed() {
    let primary = start_node("primary", vec![]).await;
    let h = harness(vec![primary], |config| {
        config.gateway.allow_signups = false;
    })
    .await;

    let (status, _, body) = h
        .post(
            "/xrpc/com.atproto.server.createAccount",
            json!({"handle": "x.rocksky.social", "email": "x@example.com", "password": "p"}),
        )
        .await;

    assert_eq!(status, 403, "{body}");
}

#[tokio::test]
async fn placement_respects_a_node_that_refuses_signups() {
    let primary = start_node("primary", vec![]).await;
    let radxa = start_node("radxa", vec![]).await;
    let h = harness(vec![primary, radxa], |config| {
        // Only radxa may take new accounts.
        config.nodes[0].accepts_signups = false;
    })
    .await;

    for i in 0..3 {
        let (status, _, body) = h
            .post(
                "/xrpc/com.atproto.server.createAccount",
                json!({
                    "handle": format!("user{i}.rocksky.social"),
                    "email": format!("user{i}@example.com"),
                    "password": "correct-horse",
                }),
            )
            .await;
        assert_eq!(status, 200, "{body}");
    }

    assert_eq!(h.nodes["primary"].created().len(), 0);
    assert_eq!(h.nodes["radxa"].created().len(), 3);
}

#[tokio::test]
async fn least_accounts_placement_spreads_signups() {
    let primary = start_node("primary", vec![]).await;
    let radxa = start_node("radxa", vec![]).await;
    let h = harness(vec![primary, radxa], |_| {}).await;

    for i in 0..4 {
        let (status, _, _) = h
            .post(
                "/xrpc/com.atproto.server.createAccount",
                json!({
                    "handle": format!("user{i}.rocksky.social"),
                    "email": format!("user{i}@example.com"),
                    "password": "correct-horse",
                }),
            )
            .await;
        assert_eq!(status, 200);
    }

    let a = h.nodes["primary"].created().len();
    let b = h.nodes["radxa"].created().len();
    assert_eq!(a + b, 4);
    assert_eq!(a, 2, "least-accounts should alternate: got {a}/{b}");
    assert_eq!(b, 2);
}

#[tokio::test]
async fn a_delegate_hop_does_not_recurse() {
    let h = fleet().await;

    // A node asking the gateway marks the request; the gateway must answer from
    // what it knows rather than asking that node back.
    let (status, _, body) = h
        .get_with(
            "/xrpc/com.atproto.identity.resolveHandle?handle=bob.rocksky.social",
            &[("x-pdsgw-delegate-hop", "1")],
        )
        .await;

    // Not yet in the registry, so it is honestly unresolvable rather than a loop.
    assert_eq!(status, 400);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["error"], json!("UnableToResolveHandle"));

    // Once the gateway knows the account, the hop is answered from the registry.
    h.state
        .store
        .upsert_account(
            &stub_did("bob.rocksky.social"),
            "bob.rocksky.social",
            "radxa",
        )
        .await
        .unwrap();

    let (status, _, body) = h
        .get_with(
            "/xrpc/com.atproto.identity.resolveHandle?handle=bob.rocksky.social",
            &[("x-pdsgw-delegate-hop", "1")],
        )
        .await;
    assert_eq!(status, 200);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["did"], json!(stub_did("bob.rocksky.social")));
}

#[tokio::test]
async fn the_firehose_can_be_turned_off() {
    let h = fleet().await;
    let (status, _, _) = h
        .get_with("/xrpc/com.atproto.sync.subscribeRepos", &[])
        .await;

    assert_eq!(status, 501);
}
