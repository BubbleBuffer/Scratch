// The bearer for a request to the hub. A device token bound to the device's
// key is refused as a bare `Authorization: Bearer <token>` (except on
// /auth/upgrade and /auth/signout): each request carries a short-lived
// session from `hubBearer` instead. When the hub answers 401 to a session,
// the session is forgotten and the request is sent once more with a fresh
// one.

import 'dart:io';

import '../../src/rust/api/sync.dart' as bridge;
import '../../src/rust/api/types.dart' show TrustedHubCertificate;
import '../../src/rust/api/vault.dart' show NotesVault;

/// Sends one hub request with the bearer for [token]. [send] makes the
/// request with the bearer it is given and returns the response; [statusOf]
/// reads the HTTP status of that response. A 401 to a session (not to the
/// bare token, which an old hub uses) forgets the session and sends again
/// with a fresh one. Errors from the bridge or from [send] propagate.
/// Without a [vault] (tests, no open vault) it sends the bare token.
Future<R> sendToHub<R>({
  required NotesVault? vault,
  required String hubUrl,
  required String token,
  required List<TrustedHubCertificate> trusted,
  required Future<R> Function(String bearer) send,
  required int Function(R response) statusOf,
}) async {
  if (vault == null) return send(token);
  final first = await bridge.hubBearer(
    vault: vault,
    hubUrl: hubUrl,
    token: token,
    trusted: trusted,
  );
  final response = await send(first);
  if (statusOf(response) != HttpStatus.unauthorized || first == token) {
    return response;
  }
  await bridge.hubBearerInvalidate(
    vault: vault,
    hubUrl: hubUrl,
    token: token,
    bearer: first,
  );
  final fresh = await bridge.hubBearer(
    vault: vault,
    hubUrl: hubUrl,
    token: token,
    trusted: trusted,
  );
  return send(fresh);
}
