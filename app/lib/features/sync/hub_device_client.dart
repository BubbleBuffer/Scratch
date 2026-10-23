// Renames this device on the hub: `POST /device/name` with its device token
// and X-Device. The hub trims the name and refuses one another computer
// with "Let the hub use this computer" on already has (ignoring case); the
// new name takes effect at once for `/mcp/agent`, and the old one stops
// working.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import '../ai/agent_token_client.dart' show AgentTokenClient;

class HubRenameException implements Exception {
  const HubRenameException(this.message);
  final String message;
  @override
  String toString() => message;
}

typedef HubDeviceRename =
    Future<String> Function({
      required String hubUrl,
      required String token,
      required String deviceId,
      required String name,
    });

/// Tests replace the hub call.
HubDeviceRename? debugRenameHubDevice;

/// Renames the device; returns the name the hub saved. Throws
/// [HubRenameException] with a message to show.
Future<String> renameHubDevice({
  required String hubUrl,
  required String token,
  required String deviceId,
  required String name,
}) {
  final debug = debugRenameHubDevice;
  if (debug != null) {
    return debug(hubUrl: hubUrl, token: token, deviceId: deviceId, name: name);
  }
  return _rename(hubUrl, token, deviceId, name);
}

Future<String> _rename(
  String hubUrl,
  String token,
  String deviceId,
  String name,
) async {
  const timeout = Duration(seconds: 20);
  final base = AgentTokenClient.hubUri(hubUrl);
  if (base == null) {
    throw const HubRenameException('This hub address is not https.');
  }
  final client = HttpClient()..connectionTimeout = timeout;
  try {
    final req = await client
        .postUrl(base.resolve('/device/name'))
        .timeout(timeout);
    req.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
    req.headers.set('X-Device', deviceId);
    req.headers.contentType = ContentType.json;
    req.write(jsonEncode({'name': name}));
    final res = await req.close().timeout(timeout);
    final text = await res.transform(utf8.decoder).join().timeout(timeout);
    Object? reply;
    try {
      reply = jsonDecode(text);
    } on FormatException {
      reply = null;
    }
    final map = reply is Map ? reply : const {};
    if (res.statusCode == 200 && map['name'] is String) {
      return map['name'] as String;
    }
    if (res.statusCode == 401 || res.statusCode == 403) {
      throw const HubRenameException(
        "The hub didn't accept this device. Sign in to the hub again.",
      );
    }
    if (map['error'] is String) throw HubRenameException(map['error'] as String);
    if (res.statusCode == 404 || res.statusCode == 405) {
      throw const HubRenameException(
        "This hub can't rename computers yet. Update the hub.",
      );
    }
    throw HubRenameException(
      "The hub couldn't rename this computer (error ${res.statusCode}).",
    );
  } on HubRenameException {
    rethrow;
  } on TimeoutException {
    throw const HubRenameException("The hub didn't answer in time.");
  } on IOException {
    throw const HubRenameException("Couldn't reach the hub.");
  } finally {
    client.close(force: true);
  }
}
