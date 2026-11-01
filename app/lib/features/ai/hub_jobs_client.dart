// Hub jobs from a signed-in device: the `jobs` tool on the hub's /mcp,
// called with this device's token and X-Device (the hub lets a device call
// that tool only). "Ask AI" and tasks assigned to the agent create a job
// about a page when the default runner is the Hub (phones: always); the
// hub writes the answer into the page, which syncs back.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import '../../src/rust/api/types.dart' show TrustedHubCertificate;
import '../../src/rust/api/vault.dart' show NotesVault;
import '../sync/hub_bearer.dart';
import 'agent_token_client.dart' show AgentTokenClient;

/// A job as the hub reports it.
class HubJob {
  const HubJob({required this.id, required this.status, this.tools, this.result, this.error});
  final String id;

  /// queued, running, completed, failed, cancelled or interrupted.
  final String status;
  final String? tools, result, error;

  factory HubJob.fromJson(Map<String, dynamic> j) => HubJob(
        id: j['id'] as String,
        status: j['status'] as String? ?? '',
        tools: j['tools'] as String?,
        result: j['result'] as String?,
        error: j['error'] as String?,
      );
}

class HubJobException implements Exception {
  const HubJobException(this.message);
  final String message;
  @override
  String toString() => message;
}

/// Creates and cancels hub jobs (tests use a fake).
abstract class HubJobs {
  /// A job about [page]; its answer goes after block [after] (an empty
  /// paragraph there is replaced) or at the page's end.
  Future<HubJob> create({required String prompt, required String page, String? after});

  /// Cancels [id]; a job that already ended is left as it is.
  Future<void> cancel(String id);
}

class HubJobsClient implements HubJobs {
  HubJobsClient({
    required this.hubUrl,
    required this.token,
    required this.deviceId,
    required this.vault,
    required this.trusted,
    HttpClient Function()? httpClient,
    this.timeout = const Duration(seconds: 20),
  }) : _httpClient = httpClient ?? HttpClient.new;

  final String hubUrl, token, deviceId;

  /// The vault whose device key proves this device (the bearer is a session).
  final NotesVault vault;

  /// What this device trusts for the hub beyond the public roots (as sync).
  final List<TrustedHubCertificate> trusted;

  final Duration timeout;
  final HttpClient Function() _httpClient;

  @override
  Future<HubJob> create({required String prompt, required String page, String? after}) async =>
      HubJob.fromJson(await _call({
        'action': 'create',
        'prompt': prompt,
        'page': page,
        'after': ?after,
      }));

  @override
  Future<void> cancel(String id) async {
    try {
      await _call({'action': 'cancel', 'job': id});
    } on HubJobException catch (e) {
      if (!e.message.contains('already ended')) rethrow;
    }
  }

  /// One `tools/call` on the hub's /mcp with [bearer]: the status and body.
  Future<(int, String)> _post(Uri base, Map<String, Object?> arguments, String bearer) async {
    final client = _httpClient()..connectionTimeout = timeout;
    try {
      final req = await client.postUrl(base.resolve('/mcp')).timeout(timeout);
      req.headers.set(HttpHeaders.authorizationHeader, 'Bearer $bearer');
      req.headers.set('X-Device', deviceId);
      req.headers.contentType = ContentType.json;
      req.write(jsonEncode({
        'jsonrpc': '2.0',
        'id': 1,
        'method': 'tools/call',
        'params': {'name': 'jobs', 'arguments': arguments},
      }));
      final res = await req.close().timeout(timeout);
      final text = await res.transform(utf8.decoder).join().timeout(timeout);
      return (res.statusCode, text);
    } finally {
      client.close(force: true);
    }
  }

  Future<Map<String, dynamic>> _call(Map<String, Object?> arguments) async {
    final base = AgentTokenClient.hubUri(hubUrl);
    if (base == null) throw const HubJobException('This hub address is not https.');
    try {
      final (status, text) = await sendToHub(
        vault: vault,
        hubUrl: hubUrl,
        token: token,
        trusted: trusted,
        send: (bearer) => _post(base, arguments, bearer),
        statusOf: (r) => r.$1,
      );
      if (status == 401 || status == 403) {
        throw const HubJobException("The hub didn't accept this device. Sign in to the hub again, or update the hub.");
      }
      if (status != 200) {
        throw HubJobException("The hub couldn't start the agent (error $status).");
      }
      final Object? reply;
      try {
        reply = jsonDecode(text);
      } on FormatException {
        throw const HubJobException('The hub answered something unexpected.');
      }
      if (reply is! Map<String, dynamic>) {
        throw const HubJobException('The hub answered something unexpected.');
      }
      final error = reply['error'];
      if (error is Map) throw HubJobException('${error['message'] ?? 'The hub refused the job.'}');
      final result = reply['result'];
      final content = result is Map ? result['content'] : null;
      final body = content is List && content.isNotEmpty && content.first is Map ? content.first['text'] : null;
      if (body is! String) throw const HubJobException('The hub answered something unexpected.');
      if (result['isError'] == true) {
        // Older hubs refuse a device on /mcp outright.
        throw HubJobException(body.contains('only the jobs tool') || body.contains('unknown tool')
            ? "This hub can't run the agent for this device yet. Update the hub."
            : body);
      }
      final job = jsonDecode(body);
      if (job is! Map<String, dynamic>) throw const HubJobException('The hub answered something unexpected.');
      return job;
    } on HubJobException {
      rethrow;
    } on TimeoutException {
      throw const HubJobException("The hub didn't answer in time.");
    } on IOException {
      throw const HubJobException("Couldn't reach the hub.");
    } catch (_) {
      // The bridge's handshake failed (hub unreachable, or the key refused).
      throw const HubJobException("Couldn't reach the hub.");
    }
  }
}
