// Hub jobs from a signed-in device: the `jobs` tool on the hub's /mcp,
// called with this device's token and X-Device (the hub lets a device call
// that tool only). "Ask AI" and tasks assigned to the agent create a job
// about a page when the default runner is the Hub (phones: always); the
// hub writes the answer into the page, which syncs back.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

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
    HttpClient Function()? httpClient,
    this.timeout = const Duration(seconds: 20),
  }) : _httpClient = httpClient ?? HttpClient.new;

  final String hubUrl, token, deviceId;
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

  Future<Map<String, dynamic>> _call(Map<String, Object?> arguments) async {
    final base = AgentTokenClient.hubUri(hubUrl);
    if (base == null) throw const HubJobException('This hub address is not https.');
    final client = _httpClient()..connectionTimeout = timeout;
    try {
      final req = await client.postUrl(base.resolve('/mcp')).timeout(timeout);
      req.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
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
      if (res.statusCode == 401 || res.statusCode == 403) {
        throw const HubJobException("The hub didn't accept this device. Sign in to the hub again, or update the hub.");
      }
      if (res.statusCode != 200) {
        throw HubJobException("The hub couldn't start the agent (error ${res.statusCode}).");
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
    } finally {
      client.close(force: true);
    }
  }
}
