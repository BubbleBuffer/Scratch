// AgentTokenClient against a loopback stub of the hub's /agent-tokens:
// the device bearer token and X-Device go on every call, the bodies match
// the hub's, and refusals become readable errors.

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/ai/agent_token_client.dart';

class _Stub {
  late HttpServer server;
  final seen = <(String, String, String?, String?, Object?)>[];
  int status = 200;
  Object? reply;

  Future<void> start() async {
    server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server.listen((req) async {
      final text = await utf8.decoder.bind(req).join();
      seen.add((
        req.method,
        req.uri.path,
        req.headers.value(HttpHeaders.authorizationHeader),
        req.headers.value('X-Device'),
        text.isEmpty ? null : jsonDecode(text),
      ));
      req.response.statusCode = status;
      if (reply != null) {
        req.response.headers.contentType = ContentType.json;
        req.response.write(jsonEncode(reply));
      }
      await req.response.close();
    });
  }

  String get url => 'http://127.0.0.1:${server.port}';
}

void main() {
  late _Stub stub;
  late AgentTokenClient client;
  setUp(() async {
    stub = _Stub();
    await stub.start();
    client = AgentTokenClient(
      hubUrl: stub.url,
      token: 'device-token-fixture',
      deviceId: 'device-1',
    );
  });
  tearDown(() => stub.server.close(force: true));

  test('list sends the device credentials and reads the reply', () async {
    stub.reply = {
      'tokens': [
        {
          'name': 'a',
          'scope': 'write',
          'created': 1,
          'expires': 5,
          'last_used': 3,
          'expired': false,
        },
        {'name': 'b', 'scope': 'read', 'created': 2, 'expires': null},
      ],
      'expiry_choices_days': [null, 30, 90, 365],
      'default_expiry_days': 90,
    };
    final list = await client.list();
    expect(stub.seen.single.$1, 'GET');
    expect(stub.seen.single.$2, '/agent-tokens');
    expect(stub.seen.single.$3, 'Bearer device-token-fixture');
    expect(stub.seen.single.$4, 'device-1');
    expect(list.tokens.map((t) => t.name), ['a', 'b']);
    expect(list.tokens.first.lastUsed, 3);
    expect(list.tokens.last.expires, isNull);
    expect(list.expiryChoices, [null, 30, 90, 365]);
    expect(list.defaultExpiry, 90);
  });

  test('create posts name, scope and expiry, and returns the secret', () async {
    stub
      ..status = 201
      ..reply = {
        'name': 'n',
        'scope': 'read',
        'created': 1,
        'expires': null,
        'last_used': null,
        'expired': false,
        'token': 'fake-secret',
      };
    final made = await client.create(name: 'n', scope: 'read', expiresDays: null);
    expect(stub.seen.single.$1, 'POST');
    expect(stub.seen.single.$5, {'name': 'n', 'scope': 'read', 'expires_days': null});
    expect(made.secret, 'fake-secret');
    expect(made.info.expires, isNull);
  });

  test('revoke deletes by name; a gone token is fine', () async {
    stub.status = 204;
    await client.revoke('old-bot');
    stub.status = 404;
    await client.revoke('old-bot');
    expect(stub.seen.map((s) => '${s.$1} ${s.$2}'), [
      'DELETE /agent-tokens/old-bot',
      'DELETE /agent-tokens/old-bot',
    ]);
  });

  test('refusals become messages', () async {
    stub
      ..status = 401
      ..reply = {'error': 'unknown token'};
    await expectLater(
      client.list(),
      throwsA(isA<AgentTokenException>().having((e) => e.status, 'status', 401)),
    );
    stub
      ..status = 409
      ..reply = {'error': 'duplicate_name'};
    await expectLater(
      client.create(name: 'x', scope: 'read', expiresDays: 30),
      throwsA(
        isA<AgentTokenException>().having(
          (e) => e.message,
          'message',
          contains('already exists'),
        ),
      ),
    );
  });

  test('only https or loopback hubs', () {
    expect(AgentTokenClient.hubUri('https://hub.example'), isNotNull);
    expect(AgentTokenClient.hubUri('http://127.0.0.1:8080'), isNotNull);
    expect(AgentTokenClient.hubUri('http://hub.example'), isNull);
    expect(AgentTokenClient.hubUri('https://hub.example/x'), isNull);
    expect(AgentTokenClient.hubUri('https://u:p@hub.example'), isNull);
  });
}
