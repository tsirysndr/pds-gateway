# `social.rocksky.auth.*`

Two-factor authentication and passkeys, as XRPC methods.

The atproto lexicon defines neither, so every PDS that offers them has invented
its own interface — atoll's are form-encoded endpoints authorised by a browser
cookie and answered with HTML. That cannot be shared: a client would need to
know which implementation it is talking to, and a token-based client cannot use
it at all.

These definitions are the shared contract instead. They are ordinary XRPC
methods, so:

- a bearer access token authorises them, like every other `com.atproto` method;
- the gateway routes them to the account's own PDS with no special handling;
- one client works against any implementation.

A PDS that does not implement them returns `MethodNotImplemented`, which is how
a client can tell, rather than guessing from a 404 on a path.

## Methods

| Method | Purpose |
| --- | --- |
| `getTwoFactor` | current state and remaining recovery codes |
| `beginTwoFactor` | start enrolment; returns the secret and `otpauth://` URI |
| `confirmTwoFactor` | confirm with a code; issues recovery codes |
| `disableTwoFactor` | turn off; needs password **and** a code |
| `regenerateRecoveryCodes` | replace the recovery codes |
| `listPasskeys` | registered credentials |
| `beginPasskeyRegistration` | WebAuthn creation options plus a `requestId` |
| `finishPasskeyRegistration` | verify the attestation and store the credential |
| `deletePasskey` | remove a credential |

## Notes on the design

`disableTwoFactor` and `regenerateRecoveryCodes` require the password as well as
a current code. Possession of an access token is not enough to take the second
factor off, or a stolen session would undo the protection it exists to provide.

`beginTwoFactor` returns a secret that is **not yet in force**. Until
`confirmTwoFactor` succeeds the account still authenticates with its password
alone, so a half-finished enrolment cannot lock anyone out.

Passkey registration is two calls because WebAuthn is a challenge-response
ceremony. The server keeps the challenge against `requestId` with a short
expiry; `finishPasskeyRegistration` is the only thing that consumes it, and it
must reject a `requestId` it did not issue to this account.

TOTP follows RFC 6238: 30-second steps, 6 digits, SHA-1, which is what
authenticator apps assume. Implementations should accept the adjacent step on
either side to allow for clock drift, and must rate-limit verification —
six digits is only 20 bits.
