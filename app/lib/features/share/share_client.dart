// The hub's share links of a page over HTTP: GET /shares?page=, POST /shares
// and DELETE /shares/{id}, with this device's token and X-Device. A link is
// public or private, and a snapshot of the page or a live one.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import '../ai/agent_token_client.dart' show AgentTokenClient;
import '../../src/rust/api/types.dart' show TrustedHubCertificate;
import '../../src/rust/api/vault.dart' show NotesVault;
import '../sync/hub_bearer.dart';

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
    this.title,
    this.existing = false,
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

  /// The page's title as the hub has it; null when the page is deleted or in
  /// the trash. Set on the lists across pages.
  final String? title;

  /// True when a create answered with a link the page already had (the hub
  /// answers 200 instead of 201).
  final bool existing;

  factory Share.fromJson(Map<String, dynamic> j) => Share(
    id: j['id'] as String,
    url: j['url'] as String,
    page: j['page'] as String,
    visibility: j['visibility'] as String,
    kind: j['kind'] as String,
    created: (j['created'] as num).toInt(),
    createdBy: j['created_by'] as String?,
    title: j['title'] as String?,
    existing: j['existing'] == true,
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
    this.vault,
    this.trusted = const [],
    HttpClient Function()? httpClient,
    this.timeout = const Duration(seconds: 15),
  }) : _httpClient = httpClient ?? HttpClient.new;

  final String hubUrl;
  final String token;
  final String deviceId;

  /// The vault whose device key proves this device (the bearer is a
  /// session); null sends the bare token.
  final NotesVault? vault;

  /// What this device trusts for the hub beyond the public roots (as sync).
  final List<TrustedHubCertificate> trusted;

  final Duration timeout;
  final HttpClient Function() _httpClient;

  /// Active share links of [pageId], newest first.
  Future<List<Share>> list(String pageId) async {
    final (status, body) = await _send(
      'GET',
      '/shares?page=${Uri.encodeQueryComponent(pageId)}',
    );
    if (status != 200) throw _error(status, body);
    return _shareList(body);
  }

  /// Every active link across pages, newest first, deleted pages included.
  Future<List<Share>> listAll() async {
    final (status, body) = await _send('GET', '/shares?all=1');
    if (status != 200) throw _error(status, body);
    return _shareList(body);
  }

  /// A link to [pageId]: a new one (201), or the page's existing link with
  /// the same choices (200, [Share.existing] true). [visibility] is `public`
  /// or `private`, [kind] `snapshot` or `live`.
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
    if (status != 201 && status != 200) throw _error(status, body);
    return Share.fromJson(body as Map<String, dynamic>);
  }

  List<Share> _shareList(Object? body) {
    final j = body as Map<String, dynamic>;
    return [
      for (final s in (j['shares'] as List? ?? const []))
        Share.fromJson(s as Map<String, dynamic>),
    ];
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
    try {
      return await sendToHub(
        vault: vault,
        hubUrl: hubUrl,
        token: token,
        trusted: trusted,
        send: (bearer) => _sendWith(base, bearer, method, path, json),
        statusOf: (r) => r.$1,
      );
    } on ShareException {
      rethrow;
    } on IOException {
      throw const ShareException("Couldn't reach the hub.");
    } catch (_) {
      // The bridge's handshake failed (hub unreachable, or the key refused).
      throw const ShareException("Couldn't reach the hub.");
    }
  }

  Future<(int, Object?)> _sendWith(
    Uri base,
    String bearer,
    String method,
    String path,
    Map<String, Object?>? json,
  ) async {
    final client = _httpClient()..connectionTimeout = timeout;
    try {
      final req = await client
          .openUrl(method, base.resolve(path))
          .timeout(timeout);
      req.headers.set(HttpHeaders.authorizationHeader, 'Bearer $bearer');
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
