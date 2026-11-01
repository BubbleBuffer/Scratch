// The loopback HTTP server of the app remote-control endpoint. It binds
// 127.0.0.1 on a random port, publishes port and token in
// `<data dir>/remote/endpoint.json` and removes that file when it stops.
// What a request does is in [RemoteHandler].

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'remote_handler.dart';

/// Requests larger than this are refused; commands are small.
const _maxBody = 1024 * 1024;

/// 64 hex characters (256 bits) from [random].
String newRemoteToken([Random? random]) {
  final r = random ?? Random.secure();
  final b = StringBuffer();
  for (var i = 0; i < 32; i++) {
    b.write(r.nextInt(256).toRadixString(16).padLeft(2, '0'));
  }
  return b.toString();
}

class RemoteControlServer {
  RemoteControlServer._(this._http, this.token, this.endpointFile, this._handler);

  final HttpServer _http;
  final String token;
  final File endpointFile;
  final RemoteHandler _handler;
  bool _stopped = false;

  int get port => _http.port;

  /// Starts serving [backend]. [dataDir] holds the `remote/` folder.
  static Future<RemoteControlServer> start({
    required RemoteBackend backend,
    required String dataDir,
    required String version,
    Random? random,
  }) async {
    final token = newRemoteToken(random);
    final handler = RemoteHandler(
      backend: backend,
      token: token,
      version: version,
      pid: pid,
    );
    final http = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final dir = Directory('$dataDir${Platform.pathSeparator}remote');
    final file = File('${dir.path}${Platform.pathSeparator}endpoint.json');
    try {
      dir.createSync(recursive: true);
      _chmod('700', dir.path);
      // Created empty and restricted before the token goes in.
      file.writeAsStringSync('');
      _chmod('600', file.path);
      file.writeAsStringSync(
        jsonEncode({
          'port': http.port,
          'token': token,
          'pid': pid,
          'version': version,
        }),
        flush: true,
      );
    } catch (_) {
      await http.close(force: true);
      rethrow;
    }
    final server = RemoteControlServer._(http, token, file, handler);
    http.listen(server._serve, onError: (Object _) {});
    return server;
  }

  /// Linux only; on Windows the per-user data folder's ACL applies.
  static void _chmod(String mode, String path) {
    if (!Platform.isLinux) return;
    final r = Process.runSync('chmod', [mode, path]);
    if (r.exitCode != 0) {
      throw FileSystemException('chmod $mode failed: ${r.stderr}', path);
    }
  }

  Future<void> stop() async {
    if (_stopped) return;
    _stopped = true;
    try {
      if (endpointFile.existsSync()) endpointFile.deleteSync();
    } catch (_) {}
    await _http.close(force: true);
  }

  Future<void> _serve(HttpRequest request) async {
    RemoteReply reply;
    try {
      final body = await _readBody(request);
      reply = body == null
          ? const RemoteReply(400, {
              'ok': false,
              'error': 'request body too large',
            })
          : await _handler.handle(
              method: request.method,
              path: request.uri.path,
              authorization: request.headers.value(
                HttpHeaders.authorizationHeader,
              ),
              body: body,
            );
    } catch (e) {
      reply = RemoteReply(500, {'ok': false, 'error': '$e'});
    }
    try {
      final bytes = Uint8List.fromList(utf8.encode(jsonEncode(reply.body)));
      request.response
        ..statusCode = reply.status
        ..persistentConnection = false
        ..headers.contentType = ContentType('application', 'json', charset: 'utf-8')
        ..headers.set(HttpHeaders.connectionHeader, 'close')
        ..contentLength = bytes.length
        ..add(bytes);
      await request.response.close();
    } catch (_) {
      // The client went away.
    }
  }

  /// The body as text, or null when it exceeds [_maxBody].
  Future<String?> _readBody(HttpRequest request) async {
    final chunks = BytesBuilder(copy: false);
    await for (final part in request) {
      chunks.add(part);
      if (chunks.length > _maxBody) return null;
    }
    return utf8.decode(chunks.takeBytes(), allowMalformed: true);
  }
}
