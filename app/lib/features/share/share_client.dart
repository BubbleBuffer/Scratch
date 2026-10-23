// The hub's share links of a page over HTTP: GET /shares?page=, POST /shares
// and DELETE /shares/{id}, with this device's token and X-Device. A link is
// public or private, and a snapshot of the page or a live one.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import '../ai/agent_token_client.dart' show AgentTokenClient;

/// A share link as the hub lists it. Unix milliseconds for [created].
class Share {
  const Share({
    required this.id,
    required this.url,
    required this.page,
    required this.visibility,
    required this.kind,
    required this.created,
    this.createdBy,
  });

  final String id;
  final String url;
  final String page;

  /// `public` or `private`.
  final String visibility;

  /// `snapshot` (a copy of the page as it is now) or `live` (the latest version).
  final String kind;
  final int created;
  final String? createdBy;

  factory Share.fromJson(Map<String, dynamic> j) => Share(
    id: j['id'] as String,
    url: j['url'] as String,
    page: j['page'] as String,
    visibility: j['visibility'] as String,
    kind: j['kind'] as String,
    created: (j['created'] as num).toInt(),
    createdBy: j['created_by'] as String?,
  );

  /// "Public · This version" or "Private · Live link".
  String get label =>
      '${visibility == 'public' ? 'Public' : 'Private'} · '
      '${kind == 'live' ? 'Live link' : 'This version'}';
}

class ShareException implements Exception {
  const ShareException(this.message, {this.status});
  final String message;
  final int? status;
  @override
  String toString() => message;
}

class ShareClient {
  ShareClient({
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

  /// Active share links of [pageId], newest first.
  Future<List<Share>> list(String pageId) async {
    final (status, body) = await _send(
      'GET',
      '/shares?page=${Uri.encodeQueryComponent(pageId)}',
    );
    if (status != 200) throw _error(status, body);
    final j = body as Map<String, dynamic>;
    return [
      for (final s in (j['shares'] as List? ?? const []))
        Share.fromJson(s as Map<String, dynamic>),
    ];
  }

  /// A new link to [pageId]. [visibility] is `public` or `private`, [kind]
  /// `snapshot` or `live`.
  Future<Share> create(
    String pageId, {
    required String visibility,
    required String kind,
  }) async {
    final (status, body) = await _send('POST', '/shares', {
      'page': pageId,
      'visibility': visibility,
      'kind': kind,
    });
    if (status != 201) throw _error(status, body);
    return Share.fromJson(body as Map<String, dynamic>);
  }

  /// Ends a link. A link that is already gone counts as ended.
  Future<void> revoke(String id) async {
    final (status, body) = await _send(
      'DELETE',
      '/shares/${Uri.encodeComponent(id)}',
    );
    if (status != 204 && status != 404) throw _error(status, body);
  }

  ShareException _error(int status, Object? body) {
    final code = body is Map ? body['error'] : null;
    final message = switch ((status, code)) {
      (400, 'databases_not_shareable') =>
        "Database pages can't be shared.",
      (404, 'no_page') => "This page isn't on the hub, so it can't be shared yet.",
      (401 || 403, _) =>
        "The hub didn't accept this device. Sign in to the hub again.",
      (404 || 405, _) => "This hub can't share pages yet. Update the hub.",
      _ => "Couldn't reach the hub's share links (error $status).",
    };
    return ShareException(message, status: status);
  }

  Future<(int, Object?)> _send(
    String method,
    String path, [
    Map<String, Object?>? json,
  ]) async {
    final base = AgentTokenClient.hubUri(hubUrl);
    if (base == null) {
      throw const ShareException('This hub address is not https.');
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
    } on ShareException {
      rethrow;
    } on TimeoutException {
      throw const ShareException("The hub didn't answer in time.");
    } on IOException {
      throw const ShareException("Couldn't reach the hub.");
    } finally {
      client.close(force: true);
    }
  }
}
