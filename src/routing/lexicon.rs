//! Which part of a request identifies the account it belongs to.

/// Where a routing subject can be read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A query-string parameter.
    Query(&'static str),
    /// A top-level field in the JSON body. Requires buffering the body, so it
    /// is never used for streaming methods.
    Body(&'static str),
    /// A dotted path into the JSON body, e.g. `subject.did`.
    BodyPath(&'static str),
    /// The `sub` claim of the bearer token, unverified. The gateway only routes
    /// on it; the upstream PDS still verifies the signature.
    AuthSubject,
}

/// How the gateway handles a method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handling {
    /// The gateway answers without contacting a node.
    Local,
    /// Every node is queried and the results merged.
    Fanout,
    /// Forwarded to one node, found via `sources`.
    Proxy { sources: &'static [Source] },
    /// Forwarded to whichever node accepts it. Used for login, where the
    /// identifier may be an email the gateway cannot resolve.
    Broadcast { sources: &'static [Source] },
}

impl Handling {
    pub fn sources(&self) -> &'static [Source] {
        match self {
            Self::Proxy { sources } | Self::Broadcast { sources } => sources,
            Self::Local | Self::Fanout => &[],
        }
    }

    /// Whether finding the subject requires reading the request body.
    pub fn needs_body(&self) -> bool {
        self.sources()
            .iter()
            .any(|s| matches!(s, Source::Body(_) | Source::BodyPath(_)))
    }
}

const AUTH: &[Source] = &[Source::AuthSubject];

const fn proxy(sources: &'static [Source]) -> Handling {
    Handling::Proxy { sources }
}

/// Methods whose body must never be buffered: blobs and repository imports can
/// be gigabytes, so they are routed on the bearer token alone.
pub fn is_streaming(nsid: &str) -> bool {
    matches!(
        nsid,
        "com.atproto.repo.uploadBlob"
            | "com.atproto.repo.importRepo"
            | "com.atproto.sync.getRepo"
            | "com.atproto.sync.getCheckout"
            | "com.atproto.sync.getBlob"
            | "com.atproto.sync.getBlocks"
    )
}

/// Methods the gateway must not forward, because answering them per-node would
/// be wrong or meaningless for a client talking to the fleet as one PDS.
pub fn classify(nsid: &str) -> Handling {
    match nsid {
        // --- answered by the gateway -------------------------------------
        "_health" | "com.atproto.server.describeServer" => Handling::Local,
        // The gateway owns the handle namespace, so it is authoritative here.
        // This is also the delegate endpoint the nodes themselves call.
        "com.atproto.identity.resolveHandle" => Handling::Local,
        // Placement plus handle reservation; handled as a special case.
        "com.atproto.server.createAccount" => Handling::Local,

        // --- merged across the fleet --------------------------------------
        "com.atproto.sync.listRepos"
        | "com.atproto.sync.listReposByCollection"
        | "com.atproto.admin.searchAccounts" => Handling::Fanout,

        // --- login: the identifier may be an email -----------------------
        "com.atproto.server.createSession" => Handling::Broadcast {
            sources: &[Source::Body("identifier")],
        },

        // --- repository reads, keyed on the repo being read ---------------
        "com.atproto.repo.getRecord"
        | "com.atproto.repo.describeRepo"
        | "com.atproto.repo.listRecords" => proxy(&[Source::Query("repo"), Source::AuthSubject]),

        // --- repository writes: the token owns the repo -------------------
        "com.atproto.repo.createRecord"
        | "com.atproto.repo.putRecord"
        | "com.atproto.repo.deleteRecord"
        | "com.atproto.repo.applyWrites" => proxy(&[Source::AuthSubject, Source::Body("repo")]),

        "com.atproto.repo.uploadBlob"
        | "com.atproto.repo.importRepo"
        | "com.atproto.repo.listMissingBlobs" => proxy(AUTH),

        // --- sync, keyed on the repo's DID -------------------------------
        "com.atproto.sync.getRepo"
        | "com.atproto.sync.getCheckout"
        | "com.atproto.sync.getHead"
        | "com.atproto.sync.getLatestCommit"
        | "com.atproto.sync.getRepoStatus"
        | "com.atproto.sync.getRecord"
        | "com.atproto.sync.getBlocks"
        | "com.atproto.sync.getBlob"
        | "com.atproto.sync.listBlobs" => proxy(&[Source::Query("did")]),

        // --- identity ----------------------------------------------------
        "com.atproto.identity.resolveDid" => proxy(&[Source::Query("did")]),
        "com.atproto.identity.resolveIdentity" => proxy(&[Source::Query("identifier")]),
        "com.atproto.identity.updateHandle"
        | "com.atproto.identity.refreshIdentity"
        | "com.atproto.identity.submitPlcOperation"
        | "com.atproto.identity.signPlcOperation"
        | "com.atproto.identity.requestPlcOperationSignature"
        | "com.atproto.identity.getRecommendedDidCredentials" => proxy(AUTH),

        // --- admin: the subject is named explicitly ----------------------
        "com.atproto.admin.getAccountInfo" | "com.atproto.admin.getSubjectStatus" => {
            proxy(&[Source::Query("did")])
        }
        "com.atproto.admin.getAccountInfos" => proxy(&[Source::Query("dids")]),
        "com.atproto.admin.deleteAccount"
        | "com.atproto.admin.updateAccountHandle"
        | "com.atproto.admin.updateAccountPassword"
        | "com.atproto.admin.updateAccountSigningKey" => proxy(&[Source::Body("did")]),
        "com.atproto.admin.updateAccountEmail"
        | "com.atproto.admin.disableAccountInvites"
        | "com.atproto.admin.enableAccountInvites" => proxy(&[Source::Body("account")]),
        "com.atproto.admin.sendEmail" => proxy(&[Source::Body("recipientDid")]),
        "com.atproto.admin.updateSubjectStatus" => proxy(&[
            Source::BodyPath("subject.did"),
            Source::BodyPath("subject.repo"),
        ]),
        "com.atproto.server.createInviteCode" => proxy(&[Source::Body("forAccount")]),

        // --- everything else belongs to the caller's own account ---------
        _ => proxy(AUTH),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gateway_answers_for_its_own_namespace() {
        assert_eq!(classify("_health"), Handling::Local);
        assert_eq!(
            classify("com.atproto.server.describeServer"),
            Handling::Local
        );
        // The delegate endpoint the PDS nodes call must never be forwarded.
        assert_eq!(
            classify("com.atproto.identity.resolveHandle"),
            Handling::Local
        );
        assert_eq!(
            classify("com.atproto.server.createAccount"),
            Handling::Local
        );
    }

    #[test]
    fn fleet_wide_listings_are_merged() {
        for nsid in [
            "com.atproto.sync.listRepos",
            "com.atproto.sync.listReposByCollection",
            "com.atproto.admin.searchAccounts",
        ] {
            assert_eq!(classify(nsid), Handling::Fanout, "{nsid}");
        }
    }

    #[test]
    fn login_is_broadcast_because_the_identifier_may_be_an_email() {
        let handling = classify("com.atproto.server.createSession");
        assert!(matches!(handling, Handling::Broadcast { .. }));
        assert_eq!(handling.sources(), &[Source::Body("identifier")]);
        assert!(handling.needs_body());
    }

    #[test]
    fn reads_route_on_the_repo_and_writes_on_the_token() {
        assert_eq!(
            classify("com.atproto.repo.getRecord").sources(),
            &[Source::Query("repo"), Source::AuthSubject]
        );
        // A write is authoritative on the token; `repo` is only a fallback.
        assert_eq!(
            classify("com.atproto.repo.createRecord").sources(),
            &[Source::AuthSubject, Source::Body("repo")]
        );
    }

    #[test]
    fn streaming_methods_are_never_body_routed() {
        for nsid in [
            "com.atproto.repo.uploadBlob",
            "com.atproto.repo.importRepo",
            "com.atproto.sync.getRepo",
            "com.atproto.sync.getBlob",
        ] {
            assert!(is_streaming(nsid), "{nsid} should stream");
            assert!(
                !classify(nsid).needs_body(),
                "{nsid} must not require a buffered body"
            );
        }
    }

    #[test]
    fn nested_admin_subjects_use_a_body_path() {
        assert_eq!(
            classify("com.atproto.admin.updateSubjectStatus").sources(),
            &[
                Source::BodyPath("subject.did"),
                Source::BodyPath("subject.repo")
            ]
        );
    }

    #[test]
    fn unknown_methods_fall_back_to_the_callers_own_account() {
        for nsid in [
            "app.bsky.actor.getPreferences",
            "com.atproto.server.getSession",
            "com.example.someones.extension",
        ] {
            assert_eq!(classify(nsid).sources(), &[Source::AuthSubject], "{nsid}");
        }
    }

    #[test]
    fn only_body_sources_ask_for_buffering() {
        assert!(!classify("com.atproto.repo.getRecord").needs_body());
        assert!(classify("com.atproto.admin.deleteAccount").needs_body());
        assert!(!classify("_health").needs_body());
    }
}
