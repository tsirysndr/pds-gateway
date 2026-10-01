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
        "the gateway owns the namespace, so it overrides the node's answer"
    );
    // The node's own facts must survive rather than being invented here.
    assert_eq!(body["inviteCodeRequired"], json!(false));
    assert_eq!(body["blobUploadLimit"], json!(5242880));
    assert_eq!(body["contact"]["email"], json!("admin@rocksky.social"));
}

#[tokio::test]
async fn reports_health_and_readiness() {
    let h = fleet().await;

    let (status, _, body) = h.get("/_gateway/health").await;
    assert_eq!(status, 200);
    assert_eq!(body["nodesTotal"], json!(2));

    let (status, _, body) = h.get("/_gateway/health/ready").await;
    assert_eq!(status, 200);
    assert_eq!(body["ready"], json!(true));

    let (status, _, body) = h.get("/xrpc/_health").await;
    assert_eq!(status, 200);
    assert!(body["version"].is_string());
}

#[tokio::test]
async fn the_admin_api_requires_its_token() {
    let h = fleet().await;

    let (status, _, _) = h.get_with("/_gateway/admin/nodes", &[]).await;
    assert_eq!(status, 401);

    let (status, _, _) = h
        .get_with(
            "/_gateway/admin/nodes",
            &[("authorization", "Bearer wrong")],
        )
        .await;
    assert_eq!(status, 401);

    let (status, _, body) = h
        .get_with(
            "/_gateway/admin/nodes",
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
            "/_gateway/admin/resolve?subject=bob.rocksky.social&live=true",
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

    let (status, _, body) = h.get_with("/_gateway/metrics", &[]).await;
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

#[tokio::test]
async fn passes_through_the_routes_the_pds_owns() {
    let h = fleet().await;

    // Health, metrics and the PDS's own pages must not be shadowed: the gateway
    // fronts a hostname the PDS already serves.
    let (status, headers, body) = h.get_with("/account/sessions", &[]).await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("\"owner\":\"pds\""));
    assert_eq!(Harness::node_header(&headers).as_deref(), Some("primary"));

    let (status, _, body) = h.get("/health").await;
    assert_eq!(status, 200);
    assert_eq!(
        body["pds"],
        json!("primary"),
        "this must be the PDS's own health"
    );

    let (status, _, body) = h.get_with("/metrics", &[]).await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("pds_up"));

    let (status, _, body) = h.get("/.well-known/did.json").await;
    assert_eq!(status, 200);
    assert_eq!(body["id"], json!("did:web:primary.rocksky.social"));
}

#[tokio::test]
async fn passes_through_a_post_with_its_body() {
    let h = fleet().await;

    let (status, _, body) = h
        .post(
            "/oauth/par",
            json!({"client_id": "https://app.example/metadata.json"}),
        )
        .await;

    assert_eq!(status, 201, "{body}");
    assert_eq!(body["request_uri"], json!("urn:primary:abc"));
}

#[tokio::test]
async fn gateway_status_lives_under_its_own_prefix() {
    let h = fleet().await;

    let (status, _, body) = h.get("/_gateway").await;
    assert_eq!(status, 200);
    assert_eq!(body["service"], json!("pds-gateway"));

    let (status, _, body) = h.get("/_gateway/health").await;
    assert_eq!(status, 200);
    assert_eq!(
        body["nodesTotal"],
        json!(2),
        "gateway health, not the PDS's"
    );
}

#[tokio::test]
async fn a_token_sends_passthrough_to_the_accounts_own_node() {
    let h = fleet().await;
    let bob = stub_did("bob.rocksky.social");
    h.state
        .store
        .upsert_account(&bob, "bob.rocksky.social", "radxa")
        .await
        .unwrap();

    let auth = format!("Bearer {}", token(&bob, None));
    let (status, headers, body) = h
        .get_with("/account/sessions", &[("authorization", &auth)])
        .await;

    assert_eq!(status, 200);
    assert_eq!(Harness::node_header(&headers).as_deref(), Some("radxa"));
    assert!(String::from_utf8_lossy(&body).contains("\"servedBy\":\"radxa\""));
}

#[tokio::test]
async fn merged_fanout_paginates_without_loss_or_repeats() {
    let primary = start_node(
        "primary",
        (0..5)
            .map(|i| hosted(&format!("p{i}.rocksky.social"), &format!("p{i}@e.com")))
            .collect(),
    )
    .await;
    let radxa = start_node(
        "radxa",
        (0..3)
            .map(|i| hosted(&format!("r{i}.rocksky.social"), &format!("r{i}@e.com")))
            .collect(),
    )
    .await;
    let h = harness(vec![primary, radxa], |_| {}).await;

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;

    for round in 0..12 {
        let uri = match &cursor {
            Some(c) => format!("/xrpc/com.atproto.sync.listRepos?limit=4&cursor={c}"),
            None => "/xrpc/com.atproto.sync.listRepos?limit=4".to_owned(),
        };
        let (status, _, body) = h.get(&uri).await;
        assert_eq!(status, 200, "round {round}: {body}");

        for repo in body["repos"].as_array().unwrap() {
            seen.push(repo["did"].as_str().unwrap().to_owned());
        }

        cursor = body["cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }

    assert!(cursor.is_none(), "pagination should terminate");
    assert_eq!(seen.len(), 8, "every repo exactly once: {seen:?}");

    let unique: std::collections::HashSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 8, "no repo may repeat across pages");
}

#[tokio::test]
async fn an_exhausted_merged_cursor_ends_cleanly() {
    let h = fleet().await;

    let (status, _, body) = h.get("/xrpc/com.atproto.sync.listRepos?limit=50").await;
    assert_eq!(status, 200);
    // Both nodes fit in one page, so there is nothing more to page through.
    assert!(body.get("cursor").is_none(), "{body}");
}

#[tokio::test]
async fn a_failing_node_keeps_its_cursor_for_the_next_page() {
    let primary = start_node(
        "primary",
        (0..4)
            .map(|i| hosted(&format!("p{i}.rocksky.social"), &format!("p{i}@e.com")))
            .collect(),
    )
    .await;
    let radxa = start_node(
        "radxa",
        (0..4)
            .map(|i| hosted(&format!("r{i}.rocksky.social"), &format!("r{i}@e.com")))
            .collect(),
    )
    .await;
    let h = harness(vec![primary, radxa], |_| {}).await;

    let (_, _, first) = h.get("/xrpc/com.atproto.sync.listRepos?limit=2").await;
    let cursor = first["cursor"].as_str().unwrap().to_owned();

    // radxa goes down mid-pagination: its records must not be silently skipped.
    h.nodes["radxa"].set_offline(true);
    let (status, _, second) = h
        .get(&format!(
            "/xrpc/com.atproto.sync.listRepos?limit=2&cursor={cursor}"
        ))
        .await;
    assert_eq!(status, 200, "{second}");

    let next = second["cursor"].as_str().unwrap();
    let decoded = pds_gateway::routing::MergedCursor::decode(next).unwrap();
    assert!(
        decoded.get("radxa").is_some(),
        "a node that failed must keep its cursor so its records are retried"
    );
}


#[tokio::test]
async fn every_forwarded_request_carries_its_origin() {
    let h = fleet().await;

    // A PDS reached over loopback derives its origin from these headers. Without
    // them it answers 301 to its public HTTPS origin instead of doing the work,
    // so every path that forwards must set them — not just the streaming one.
    let _ = h
        .get("/xrpc/com.atproto.repo.getRecord?repo=alice.rocksky.social&collection=c&rkey=1")
        .await;
    let _ = h.get("/xrpc/com.atproto.server.describeServer").await;
    let _ = h.get("/xrpc/com.atproto.sync.listRepos").await;
    let _ = h
        .post(
            "/xrpc/com.atproto.server.createSession",
            json!({"identifier": "alice@example.com", "password": "correct-horse"}),
        )
        .await;
    let _ = h
        .post(
            "/xrpc/com.atproto.server.createAccount",
            json!({"handle": "origin.rocksky.social", "email": "o@e.com", "password": "correct-horse"}),
        )
        .await;
    let _ = h.get("/account/sessions").await;

    let mut paths = std::collections::HashSet::new();
    for node in h.nodes.values() {
        for (path, host, proto) in node.seen() {
            assert_eq!(
                proto.as_deref(),
                Some("https"),
                "{path} reached the node without x-forwarded-proto"
            );
            // Each node is addressed by its own public host.
            assert!(
                host.as_deref().is_some_and(|h| h.ends_with("rocksky.social")),
                "{path} reached the node with host {host:?}"
            );
            paths.insert(path);
        }
    }

    // Each of these travels a different code path inside the gateway: streaming
    // forward, buffered forward, fan-out, broadcast and passthrough.
    for expected in [
        "/xrpc/com.atproto.repo.getRecord",
        "/xrpc/com.atproto.server.describeServer",
        "/xrpc/com.atproto.sync.listRepos",
        "/xrpc/com.atproto.server.createSession",
        "pds-owned",
    ] {
        assert!(paths.contains(expected), "{expected} was never forwarded; saw {paths:?}");
    }
}


#[tokio::test]
async fn a_relayed_claim_does_not_make_the_relay_the_host() {
    // radxa hosts the account; primary only answers for it, the way a PDS acting
    // as a delegate relays its own delegates' answers. Trusting whoever replied
    // would send every request for this account to the wrong node.
    let primary = start_node("primary", vec![hosted("local.rocksky.social", "l@e.com")]).await;
    let radxa = start_node("radxa", vec![hosted("remote.rocksky.social", "r@e.com")]).await;
    primary.relay(&stub_did("remote.rocksky.social"), "remote.rocksky.social");

    let h = harness(vec![primary, radxa], |_| {}).await;

    let (status, _, body) = h
        .get("/xrpc/com.atproto.identity.resolveHandle?handle=remote.rocksky.social")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["did"], json!(stub_did("remote.rocksky.social")));

    // The DID document, not the answering node, decides where requests go.
    let (status, headers, body) = h
        .get("/xrpc/com.atproto.repo.getRecord?repo=remote.rocksky.social&collection=c&rkey=1")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        Harness::node_header(&headers).as_deref(),
        Some("radxa"),
        "the host is radxa; primary merely relayed the claim"
    );
    assert_eq!(body["servedBy"], json!("radxa"));

    let account = h
        .state
        .store
        .account_by_handle("remote.rocksky.social")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.node, "radxa", "the registry must record the real host");
}


#[tokio::test]
async fn the_console_is_served_from_its_own_mount() {
    let h = fleet().await;

    let (status, headers, body) = h.get_with("/console", &[]).await;
    assert_eq!(status, 200);
    assert!(
        headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(String::from_utf8_lossy(&body).contains("<div id=\"root\">"));

    // A path inside the console still serves the page, so its own routing works.
    let (status, _, body) = h.get_with("/console/settings", &[]).await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("<div id=\"root\">"));
}

#[tokio::test]
async fn the_console_answers_the_root_page() {
    let h = fleet().await;

    // The console is deliberately in front here: its sign-in resolves the
    // handle to the account's own node, which the PDS's page cannot do.
    let (status, headers, body) = h.get_with("/", &[]).await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("<div id=\"root\">"));
    assert!(Harness::node_header(&headers).is_none());

    // The PDS's own home page is still reachable where it is not shadowed; see
    // taking_over_sign_in_leaves_every_other_path_with_the_pds.
}

#[tokio::test]
async fn the_server_list_only_publishes_reachable_urls() {
    let h = fleet().await;

    let (status, _, body) = h.get("/_gateway/pds").await;
    assert_eq!(status, 200);

    let servers = body["servers"].as_array().unwrap();
    assert_eq!(servers.len(), 2);

    for server in servers {
        let url = server["url"].as_str().unwrap();
        // A browser must never be handed an address only the gateway can reach.
        assert!(
            !url.contains("127.0.0.1") && !url.contains("localhost"),
            "published an unreachable url: {url}"
        );
    }

    assert_eq!(body["handleDomains"], json!(["rocksky.social"]));
}

#[tokio::test]
async fn a_node_behind_the_gateway_is_published_as_the_gateway() {
    let primary = start_node("primary", vec![]).await;
    let radxa = start_node("radxa", vec![]).await;
    let h = harness(vec![primary, radxa], |config| {
        // The production shape: the PDS on this machine is reached over
        // loopback but publishes the gateway's own hostname.
        config.nodes[0].public_host = Some("rocksky.social".to_owned());
    })
    .await;

    let (status, _, body) = h.get("/_gateway/pds").await;
    assert_eq!(status, 200);

    let servers = body["servers"].as_array().unwrap();
    let local = servers.iter().find(|s| s["name"] == json!("primary")).unwrap();

    // Choosing it sends the browser to the gateway, which forwards to that node.
    assert_eq!(local["url"], json!("https://rocksky.social"));
}

#[tokio::test]
async fn the_gateways_own_scheme_and_port_are_kept() {
    let primary = start_node("primary", vec![]).await;
    let h = harness(vec![primary], |config| {
        // A development gateway on a port, reached over plain http.
        config.server.public_url = url::Url::parse("http://localhost:4600").unwrap();
        config.nodes[0].public_host = Some("localhost:4600".to_owned());
    })
    .await;

    let (status, _, body) = h.get("/_gateway/pds").await;
    assert_eq!(status, 200);

    let url = body["servers"][0]["url"].as_str().unwrap();
    // Not https, and not a bare host: exactly how this gateway is reached.
    assert_eq!(url, "http://localhost:4600");
}

#[tokio::test]
async fn the_console_can_be_disabled() {
    let primary = start_node("primary", vec![]).await;
    let h = harness(vec![primary], |config| {
        config.ui.enabled = false;
    })
    .await;

    // With the console off, the mount falls through to the PDS like any path.
    let (status, _, _) = h.get_with("/console", &[]).await;
    assert_eq!(status, 404, "the stub PDS has no /console");
}


#[tokio::test]
async fn the_console_answers_sign_in_and_sign_up() {
    let h = fleet().await;

    for path in ["/", "/account/login", "/account/signup"] {
        let (status, headers, body) = h.get_with(path, &[]).await;
        assert_eq!(status, 200, "{path}");
        assert!(
            String::from_utf8_lossy(&body).contains("<div id=\"root\">"),
            "{path} should be the console, not the PDS page"
        );
        // Served by the gateway itself, so no node header.
        assert!(Harness::node_header(&headers).is_none(), "{path}");
    }
}

#[tokio::test]
async fn taking_over_sign_in_leaves_every_other_path_with_the_pds() {
    let h = fleet().await;

    // Neighbours of the paths the console answers, and the PDS's own assets.
    for path in [
        "/account/sessions",
        "/account/security",
        "/oauth/authorize",
        "/assets/account.js",
    ] {
        let (status, headers, body) = h.get_with(path, &[]).await;
        assert_eq!(status, 200, "{path}");
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("\"owner\":\"pds\""), "{path} was taken over: {text}");
        assert_eq!(
            Harness::node_header(&headers).as_deref(),
            Some("primary"),
            "{path} should have been forwarded"
        );
    }
}

#[tokio::test]
async fn console_assets_load_from_any_mounted_path() {
    let h = fleet().await;

    // The page is served at several paths, so its assets cannot be relative.
    let (_, _, body) = h.get_with("/account/login", &[]).await;
    let html = String::from_utf8_lossy(&body).to_string();
    let asset = html
        .split('"')
        .find(|part| part.starts_with("/_gateway/console/assets/") && part.ends_with(".js"))
        .expect("the page should reference an absolute asset url");

    let (status, headers, _) = h.get_with(asset, &[]).await;
    assert_eq!(status, 200, "{asset} did not load");
    assert!(
        headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("javascript")
    );
}

#[tokio::test]
async fn a_missing_console_asset_is_not_served_as_the_page() {
    let h = fleet().await;

    // A 200 of HTML here would make a broken bundle look like a working one.
    let (status, _, _) = h
        .get_with("/_gateway/console/assets/does-not-exist.js", &[])
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn the_screens_the_console_answers_are_configurable() {
    let primary = start_node("primary", vec![hosted("alice.rocksky.social", "alice@example.com")]).await;
    let h = harness(vec![primary], |config| {
        // Hand sign-in back to the PDS.
        config.ui.screens = vec!["/console-only".to_owned()];
    })
    .await;

    let (status, headers, body) = h.get_with("/account/login", &[]).await;
    assert_eq!(status, 200);
    assert!(
        String::from_utf8_lossy(&body).contains("\"owner\":\"pds\""),
        "with sign-in handed back, the PDS must answer it"
    );
    assert_eq!(Harness::node_header(&headers).as_deref(), Some("primary"));

    let (status, _, body) = h.get_with("/console-only", &[]).await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("<div id=\"root\">"));
}
