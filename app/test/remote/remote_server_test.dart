import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/remote/remote_server.dart';

import 'fake_backend.dart';

void main() {
  late Directory dir;
  late RemoteControlServer server;
  late FakeBackend backend;

  setUp(() async {
    dir = Directory.systemTemp.createTempSync('remote-test-');
    backend = FakeBackend();
    server = await RemoteControlServer.start(
      backend: backend,
      dataDir: dir.path,
      version: '9.9.9',
    );
  });

  tearDown(() async {
    await server.stop();
    dir.deleteSync(recursive: true);
  });

  Future<(int, Map<String, Object?>)> call(
    String method,
    String path, {
    String? token,
    String body = '{}',
  }) async {
    final client = HttpClient();
    try {
      final req = await client.openUrl(
        method,
        Uri.parse('http://127.0.0.1:${server.port}$path'),
      );
      if (token != null) {
        req.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
      }
      req.write(body);
      final res = await req.close();
      final text = await res.transform(utf8.decoder).join();
      expect(res.headers.contentType?.mimeType, 'application/json');
      expect(res.contentLength, utf8.encode(text).length);
      return (res.statusCode, jsonDecode(text) as Map<String, Object?>);
    } finally {
      client.close(force: true);
    }
  }

  test('endpoint.json holds port, token, pid and version', () {
    final file = File('${dir.path}${Platform.pathSeparator}remote'
        '${Platform.pathSeparator}endpoint.json');
    expect(file.existsSync(), isTrue);
    final j = jsonDecode(file.readAsStringSync()) as Map<String, Object?>;
    expect(j['port'], server.port);
    expect(j['token'], matches(RegExp(r'^[0-9a-f]{64}$')));
    expect(j['token'], server.token);
    expect(j['pid'], pid);
    expect(j['version'], '9.9.9');
  });

  test('tokens differ between starts', () {
    expect(newRemoteToken(), isNot(newRemoteToken()));
    expect(newRemoteToken(), matches(RegExp(r'^[0-9a-f]{64}$')));
  });

  test('stop removes endpoint.json and closes the port', () async {
    final file = server.endpointFile;
    final port = server.port;
    await server.stop();
    expect(file.existsSync(), isFalse);
    await expectLater(
      Socket.connect(InternetAddress.loopbackIPv4, port),
      throwsA(isA<SocketException>()),
    );
  });

  test('a round trip over HTTP', () async {
    final (status, body) = await call(
      'POST',
      '/v1/status',
      token: server.token,
    );
    expect(status, 200);
    expect(body['ok'], true);
    expect(body['version'], '9.9.9');
    expect(body['pid'], pid);
    expect(backend.calls, ['status']);

    final (tapStatus, tapBody) = await call(
      'POST',
      '/v1/tap',
      token: server.token,
      body: jsonEncode({'id': 'save'}),
    );
    expect(tapStatus, 200);
    expect(tapBody, {'ok': true});
    expect(backend.calls, ['status', 'tap save']);
  });

  test('HTTP errors carry the JSON error body', () async {
    expect((await call('POST', '/v1/status')).$1, 401);
    expect((await call('POST', '/v1/status', token: 'nope')).$1, 401);
    final (getStatus, getBody) = await call(
      'GET',
      '/v1/status',
      token: server.token,
    );
    expect(getStatus, 405);
    expect(getBody['ok'], false);
    expect((await call('POST', '/v1/nope', token: server.token)).$1, 404);
    expect(
      (await call('POST', '/v1/status', token: server.token, body: '{bad')).$1,
      400,
    );
    expect(backend.calls, isEmpty);
  });
}
