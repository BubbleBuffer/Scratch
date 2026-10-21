// The hub's agent tokens over HTTP: GET/POST /agent-tokens and
// DELETE /agent-tokens/{name}, with this device's token and X-Device.
// The hub refuses agent tokens and OAuth JWTs on these routes, so any
// signed-in device manages them and nothing else can.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

/// A token as the hub lists it. The secret is never part of it.
class AgentTokenInfo {
  const AgentTokenInfo({
    required this.name,
    required this.scope,
    required this.created,
    this.expires,
    this.lastUsed,
    this.expired = false,
  });

  final String name;

  /// The tool set: `read`, `write` or `everything`.
  final String scope;

  /// Unix milliseconds.
  final int created;
  final int? expires;
  final int? lastUsed;
  final bool expired;

  factory AgentTokenInfo.fromJson(Map<String, dynamic> j) => AgentTokenInfo(
    name: j['name'] as String,
    scope: j['scope'] as String,
    created: (j['created'] as num).toInt(),
    expires: (j['expires'] as num?)?.toInt(),
    lastUsed: (j['last_used'] as num?)?.toInt(),
    expired: j['expired'] == true,
  );
}

/// The list and the lifetimes the hub offers (days, `null` for never).
class AgentTokenList {
  const AgentTokenList(this.tokens, this.expiryChoices, this.defaultExpiry);
  final List<AgentTokenInfo> tokens;
  final List<int?> expiryChoices;
  final int? defaultExpiry;
}

/// A token just created, with its secret. Show it once; it is not stored.
class CreatedAgentToken {
  const CreatedAgentToken(this.info, this.secret);
  final AgentTokenInfo info;
  final String secret;
}

class AgentTokenException implements Exception {
  const AgentTokenException(this.message, {this.status});
  final String message;
  final int? status;
  @override
  String toString() => message;
}

/// Lifetimes offered when the hub does not say (older hubs never do).
const agentTokenExpiryChoices = <int?>[null, 30, 90, 365];
const agentTokenDefaultExpiry = 90;

/// Token names the hub accepts: 1 to 40 of a-z, 0-9 and -.
final agentTokenNamePattern = RegExp(r'^[a-z0-9-]{1,40}$');

class AgentTokenClient {
  AgentTokenClient({
    required this.hubUrl,
    required this.token,
    required this.deviceId,
    HttpClient Function()? httpClient,
    this.timeout = const Duration(seconds: 15),
  }) : _httpClient = httpClient ?? HttpClient.new;

  final String hubUrl;
  final String token;
  final String deviceId;
  final Duration timeout;
  final HttpClient Function() _httpClient;

  /// https, or plain http on loopback (a local test hub), with no path.
  static Uri? hubUri(String value) {
    final uri = Uri.tryParse(value.trim());
    if (uri == null || uri.host.isEmpty || uri.userInfo.isNotEmpty) {
      return null;
    }
    final loopback =
        uri.scheme == 'http' &&
        (uri.host == '127.0.0.1' || uri.host == 'localhost');
    if (uri.scheme != 'https' && !loopback) return null;
    if (uri.hasQuery || uri.hasFragment) return null;
    if (uri.path.isNotEmpty && uri.path != '/') return null;
    return uri.replace(path: '');
  }

  Future<AgentTokenList> list() async {
    final (status, body) = await _send('GET', '/agent-tokens');
    if (status != 200) throw _error(status, body);
    final j = body as Map<String, dynamic>;
    final choices = (j['expiry_choices_days'] as List?)
        ?.map((d) => (d as num?)?.toInt())
        .toList();
    return AgentTokenList(
      [
        for (final t in (j['tokens'] as List? ?? const []))
          AgentTokenInfo.fromJson(t as Map<String, dynamic>),
      ],
      choices ?? agentTokenExpiryChoices,
      j.containsKey('default_expiry_days')
          ? (j['default_expiry_days'] as num?)?.toInt()
          : agentTokenDefaultExpiry,
    );
  }

  /// `expiresDays` null never expires.
  Future<CreatedAgentToken> create({
    required String name,
    required String scope,
    required int? expiresDays,
  }) async {
    final (status, body) = await _send('POST', '/agent-tokens', {
      'name': name,
      'scope': scope,
      'expires_days': expiresDays,
    });
    if (status != 201) throw _error(status, body);
    final j = body as Map<String, dynamic>;
    return CreatedAgentToken(AgentTokenInfo.fromJson(j), j['token'] as String);
  }

  Future<void> revoke(String name) async {
    final (status, body) = await _send(
      'DELETE',
      '/agent-tokens/${Uri.encodeComponent(name)}',
    );
    if (status != 204 && status != 404) throw _error(status, body);
  }

  AgentTokenException _error(int status, Object? body) {
    final code = body is Map ? body['error'] : null;
    final message = switch ((status, code)) {
      (409, _) => 'A token with this name already exists.',
      (400, 'invalid_name') =>
        'Use 1 to 40 lowercase letters, digits and dashes for the name.',
      (400, 'invalid_scope') =>
        "This hub doesn't know this tool set yet. Update the hub.",
      (401 || 403, _) =>
        "The hub didn't accept this device. Sign in to the hub again.",
      (404 || 405, _) =>
        "This hub can't manage agent tokens yet. Update the hub.",
      (503, _) => "The hub's token file can't be used right now.",
      _ => "Couldn't reach the hub's agent tokens (error $status).",
    };
    return AgentTokenException(message, status: status);
  }

  Future<(int, Object?)> _send(
    String method,
    String path, [
    Map<String, Object?>? json,
  ]) async {
    final base = hubUri(hubUrl);
    if (base == null) {
      throw const AgentTokenException('This hub address is not https.');
    }
    final client = _httpClient()..connectionTimeout = timeout;
    try {
      final req = await client
          .openUrl(method, base.resolve(path))
          .timeout(timeout);
      req.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
      req.headers.set('X-Device', deviceId);
      if (json != null) {
        req.headers.contentType = ContentType.json;
        req.write(jsonEncode(json));
      }
      final res = await req.close().timeout(timeout);
      final text = await res.transform(utf8.decoder).join().timeout(timeout);
      Object? body;
      try {
        body = text.isEmpty ? null : jsonDecode(text);
      } on FormatException {
        body = null;
      }
      return (res.statusCode, body);
    } on AgentTokenException {
      rethrow;
    } on TimeoutException {
      throw const AgentTokenException("The hub didn't answer in time.");
    } on IOException {
      throw const AgentTokenException("Couldn't reach the hub.");
    } finally {
      client.close(force: true);
    }
  }
}
