---
type: architecture
summary: "Android's Hub-only agent, authenticated vault sync, local acceptance tests and phone setup."
parent: "[[architecture/index]]"
status: current
---

# Android agent through the Hub

Implemented on `mobile-agent` on 2026-10-11. Android always uses the Hub agent; the local engine remains Windows/Linux only. Desktop host selection and local-agent settings are unchanged. iOS and the desktop relay (stage 4d) are outside this change.

The default Hub address is `https://notes.example.com`. Settings > AI > Agent exposes the editable address, browser sign-in/sign-out, the returned account (or the device label for an older Hub), sync status, agent opening status and the Hub's model list. The signed-out chat offers Sign in. New chat becomes available after sign-in. The existing chat, outline and note-reference cards remain in place, along with the merged mobile dock and swipe panels.

## Login and notes

`HubAuthService` uses the existing Authentik browser flow and `notes://auth` redirect. Redeeming its single-use code yields a device token. The device saves that token and connects `NotesSync` to the same Hub address, using its vault device ID. No new sync protocol or relay is required: the existing authenticated binary sync writes the Hub's own vault replica, which the agent tools read and edit.

Before sending an Android agent turn, the app waits for pending editor saves, flushes the vault and completes a sync round. A failed sync prevents the turn and reports an error, so the agent is not asked to work on notes that have not reached it. A completed agent run also requests a sync round to bring edits back; the ordinary background sync loop continues to run. The Agent settings show progress and errors, with Sync now and an agent Retry connection action.

Changing the Hub URL/token or signing out clears cached mobile chat IDs, messages, models and outline state before reopening. Signing out disconnects sync and revokes the device token where reachable; the local notes remain. Login/transport tests use only synthetic tokens and loopback servers. Release Android connections require HTTPS; debug Android additionally permits HTTP loopback for the local test harness.

`/auth/redeem` now includes an optional account display string derived from the verified OIDC user's email or subject. Older deployed Hubs remain compatible and show the registered device label instead. The account is display information, not an authorization decision.

## Local verification

- Android platform-override widget tests cover signed-out/sign-in/settings, signed-in New chat over the actual Rust bridge, token and device headers, sign-out cleanup, failed-sync turn refusal and phone system/keyboard insets.
- Six new phone-sized goldens cover signed-out chat and signed-out/signed-in settings in light and dark. Their account/model/connection state is synthetic.
- `crates/hub/tests/local_workflow.rs` starts the Hub and a mock OIDC issuer within the test. One test checks the account/token redemption response. The second obtains a test token through login, pushes a device page and checks its presence in the Hub replica, opens `/agent/ws`, sends a turn to a stub model whose script calls `search_notes`, `read_page` and `append_blocks`, then syncs the edit back and compares Hub/device contents. The servers and sockets stop within the test.
- Run `cargo test -p notes_hub`, `cargo build --release -p notes_bridge`, `flutter analyze` and the Flutter tests with `--concurrency=1`. Bridge widget tests load the rebuilt `target/release/notes_bridge.dll`.

These tests establish local wiring and note-tool behavior. They do not establish production Authentik login, provider availability, phone deep-link behavior or a real model turn. No production credentials, deployment or restart were used for this change. The remote bridge currently returns an empty chat outline; the existing outline UI is preserved, but remote outline generation needs a later Hub RPC extension. Existing note-reference rendering is retained.

### Results observed on 2026-10-11

| Check | Result |
| --- | --- |
| `cargo test -p notes_hub -j 4` | 16 passed, including both local workflow tests. |
| `cargo build --release -p notes_bridge -j 4` | Passed; bridge widget tests used the rebuilt release DLL. |
| `flutter analyze` | No issues. |
| `flutter test --concurrency=1 test/mobile_agent_settings_test.dart test/mobile_agent_connection_test.dart test/mobile_agent_golden_test.dart` | 20 passed against the final mobile source. Six phone goldens visually reviewed. |
| Existing agent, panel and AI desktop goldens (`--plain-name desktop`) | 17 passed; existing assertions and baselines unchanged. |
| Combined final Android/settings/auth/connection/desktop-host tests | 35 passed, including the final inset test and unchanged Hub auth, sign-in settings, connection and desktop host assertions. |
| Full fast suite (`--concurrency=1 -x 'golden \|\| perf \|\| drive'`) | 551 passed, 6 skipped, 3 failed in the run started before the final route/inset-test edits. Two failures reproduce individually on unchanged master: `home_first_run_test.dart`'s slash-menu heading count and `shell_test.dart`'s search-result snippet. The new inset test is green in the final focused run above. The broad suite is not claimed green. |
| `flutter build apk --debug --target-platform android-arm64` | Passed after the final code edits. Local debug APK at `app/build/app/outputs/flutter-apk/app-debug.apk`; not published or a release-signed update. |
| Rust formatting | Changed Hub files pass `rustfmt --check`; whole-crate `cargo fmt --check -p notes_hub` reports an existing formatting difference in untouched `src/config.rs`. |

All Flutter runs used one test worker. No existing assertion or timeout was changed. No test server was left running. The branch's implementation commits are `1e5e741` (Hub account and local E2E) and `7bb5882` (Android UI, settings, connection and sync wiring).

## What to do on the phone

1. Install an update built from `mobile-agent` through the normal release-signing/update path, preserving the existing app's data. This change does not publish an APK or update the deployed Hub.
2. Open Pages from the bottom dock, then the settings gear, AI and Agent. The Agent panel's signed-out Sign in action also opens this section.
3. Keep `https://notes.example.com` (or enter the intended Hub address) and tap Sign in. In the browser, sign in to Authentik with an account in the `notes-users` group. Return to Scratch via `notes://auth`.
4. Wait for the sync status to show a successful sync. Check any displayed error before proceeding; Sync now retries the existing connection. If the agent did not open, use Retry connection. Select a model if the Hub lists them.
5. Open the Agent panel, tap New chat and ask it to find/read a known note. Then ask for a small edit. Once the turn completes and sync succeeds, open the note and verify the edit. This is the outstanding production acceptance check.

The Hub must already have a usable model/provider configured. A release of the Hub's new optional account field is needed to show the account rather than the older device-label fallback, but it is not needed for sync or agent transport.
