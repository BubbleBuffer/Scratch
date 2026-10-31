import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/remote/remote_handler.dart';

import 'fake_backend.dart';

const _token =
    '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef';

void main() {
  late FakeBackend backend;
  late RemoteHandler handler;

  setUp(() {
    backend = FakeBackend();
    handler = RemoteHandler(
      backend: backend,
      token: _token,
      version: '1.2.3',
      pid: 42,
    );
  });

  Future<RemoteReply> post(
    String command, {
    Object? body = const <String, Object?>{},
    String? auth = 'Bearer $_token',
    String method = 'POST',
    String? rawBody,
  }) => handler.handle(
    method: method,
    path: '/v1/$command',
    authorization: auth,
    body: rawBody ?? jsonEncode(body),
  );

  group('auth and routing', () {
    test('a missing token is 401 and runs nothing', () async {
      final r = await post('status', auth: null);
      expect(r.status, 401);
      expect(r.body['ok'], false);
      expect(r.body['error'], isA<String>());
      expect(backend.calls, isEmpty);
    });

    test('a wrong token is 401', () async {
      for (final auth in [
        'Bearer ${_token.replaceRange(63, 64, '0')}',
        'Bearer short',
        'Bearer $_token' 'x',
        'Basic $_token',
        _token,
      ]) {
        expect((await post('status', auth: auth)).status, 401, reason: auth);
      }
      expect(backend.calls, isEmpty);
    });

    test('the right token passes', () async {
      expect((await post('status')).status, 200);
    });

    test('GET is 405', () async {
      final r = await post('status', method: 'GET');
      expect(r.status, 405);
      expect(r.body['ok'], false);
    });

    test('an unknown command is 404', () async {
      final r = await post('explode');
      expect(r.status, 404);
      expect(r.body['ok'], false);
    });

    test('a path outside /v1 is 404', () async {
      final r = await handler.handle(
        method: 'POST',
        path: '/status',
        authorization: 'Bearer $_token',
        body: '{}',
      );
      expect(r.status, 404);
    });

    test('a body that is not JSON, or not an object, is 400', () async {
      expect((await post('status', rawBody: '{nope')).status, 400);
      expect((await post('status', rawBody: '[1]')).status, 400);
    });

    test('an empty body is the same as {}', () async {
      expect((await post('status', rawBody: '')).status, 200);
    });

    test('a backend failure is 500 with the message', () async {
      backend.failWith = StateError('boom');
      final r = await post('tree');
      expect(r.status, 500);
      expect(r.body['ok'], false);
      expect(r.body['error'], contains('boom'));
    });

    test('compares tokens of different length without throwing', () {
      expect(constantTimeEquals('abc', 'abcd'), isFalse);
      expect(constantTimeEquals('', ''), isTrue);
      expect(constantTimeEquals('abc', 'abc'), isTrue);
    });
  });

  group('commands', () {
    test('status adds version and pid to the backend answer', () async {
      final r = await post('status');
      expect(r.body, {
        'ok': true,
        'version': '1.2.3',
        'pid': 42,
        'vault': {'open': true, 'name': 'vault'},
        'page': {'id': 'p1', 'title': 'Plans'},
      });
    });

    test('open returns the page; a miss is 404; no page is 400', () async {
      final ok = await post('open', body: {'page': 'Plans'});
      expect(ok.status, 200);
      expect(ok.body['page'], {'id': 'p1', 'title': 'Plans'});
      expect(backend.calls, ['open Plans']);
      expect((await post('open', body: {'page': 'missing'})).status, 404);
      expect((await post('open')).status, 400);
      expect((await post('open', body: {'page': 3})).status, 400);
    });

    test('navigate accepts the routes and lists them otherwise', () async {
      for (final route in ['home', 'settings', 'search', 'back', 'page/abc']) {
        final r = await post('navigate', body: {'route': route});
        expect(r.status, 200, reason: route);
        expect(r.body['route'], route);
      }
      expect(backend.calls, [
        'navigate home',
        'navigate settings',
        'navigate search',
        'navigate back',
        'navigate page/abc',
      ]);
      final bad = await post('navigate', body: {'route': 'nowhere'});
      expect(bad.status, 400);
      expect(bad.body['error'], contains('home'));
      expect(bad.body['error'], contains('page/<id>'));
      expect((await post('navigate', body: {'route': 'page/'})).status, 400);
    });

    test('screenshot returns size and base64 PNG; ratio defaults to 1', () async {
      final r = await post('screenshot');
      expect(r.status, 200);
      expect(r.body['width'], 2);
      expect(r.body['height'], 3);
      expect(base64Decode(r.body['png_base64']! as String), [1, 2, 3]);
      await post('screenshot', body: {'pixel_ratio': 2});
      expect(backend.calls, ['screenshot 1.0', 'screenshot 2.0']);
      expect(
        (await post('screenshot', body: {'pixel_ratio': 0})).status,
        400,
      );
      expect(
        (await post('screenshot', body: {'pixel_ratio': 'x'})).status,
        400,
      );
    });

    test('tree returns the nodes', () async {
      final r = await post('tree');
      expect(r.status, 200);
      final nodes = r.body['nodes']! as List;
      expect(nodes, hasLength(1));
      expect((nodes.single as Map)['id'], 'save');
      expect((nodes.single as Map)['rect'], {
        'x': 1.0,
        'y': 2.0,
        'w': 3.0,
        'h': 4.0,
      });
    });

    test('tap needs an id; an unknown id is 404', () async {
      final ok = await post('tap', body: {'id': 'save'});
      expect(ok.status, 200);
      expect(ok.body, {'ok': true});
      expect(backend.calls, ['tap save']);
      expect((await post('tap', body: {'id': 'gone'})).status, 404);
      expect((await post('tap')).status, 400);
    });

    test('focus returns the focused state; unknown id is 404', () async {
      final ok = await post('focus', body: {'id': 'save'});
      expect(ok.status, 200);
      expect(ok.body, {'ok': true, 'focused': true});
      expect(backend.calls, ['focus save']);
      expect((await post('focus', body: {'id': 'gone'})).status, 404);
      expect((await post('focus')).status, 400);
    });

    test('type passes text and replace (default true)', () async {
      expect(
        (await post('type', body: {'id': 'f', 'text': 'hi'})).status,
        200,
      );
      expect(
        (await post(
          'type',
          body: {'id': 'f', 'text': '', 'replace': false},
        )).status,
        200,
      );
      expect(backend.calls, ['type f "hi" replace=true', 'type f "" replace=false']);
      expect((await post('type', body: {'id': 'f'})).status, 400);
      expect((await post('type', body: {'text': 'x'})).status, 400);
      expect(
        (await post('type', body: {'id': 'f', 'text': 'x', 'replace': 1})).status,
        400,
      );
    });

    test('key parses the combo', () async {
      expect((await post('key', body: {'combo': 'Ctrl+K'})).status, 200);
      expect((await post('key', body: {'combo': 'f12'})).status, 200);
      expect(backend.calls, ['key ctrl+k', 'key f12']);
      for (final bad in ['', 'ctrl+', 'hyper+k', 'ctrl+kk', 'f13', 'f0']) {
        expect(
          (await post('key', body: {'combo': bad})).status,
          400,
          reason: bad,
        );
      }
    });
  });
}
