// Exercise Android's host selection and New chat through the real Rust bridge.
// The WebSocket peer is synthetic and local; no model or live Hub is contacted.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/features/sync/hub_auth.dart';
import 'package:notes_app/features/sync/sync_control.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';
import 'sync_helpers.dart';

// Widget tests otherwise replace Dart HTTP with a client that always returns
// 400. Only the bounded loopback auth fixture uses real Dart HTTP here.
class _LoopbackHttp extends HttpOverrides {}

Future<T> _localHttp<T>(Future<T> Function() f) =>
    HttpOverrides.runWithHttpOverrides(f, _LoopbackHttp());

class _StubHub {
  _StubHub(this.server, this.device);
  final HttpServer server;
  final String device;
  final sockets = <WebSocket>[];
  final subscriptions = <StreamSubscription<dynamic>>[];
  final calls = <String>[];
  final sessions = <Map<String, dynamic>>[];
  int redemptions = 0, signouts = 0;
  late final StreamSubscription<HttpRequest> requests;
  String get url => 'http://127.0.0.1:${server.port}';

  static Future<_StubHub> start(String device) async {
    final hub = _StubHub(
      await HttpServer.bind(InternetAddress.loopbackIPv4, 0),
      device,
    );
    hub.requests = hub.server.listen(hub._accept);
    return hub;
  }

  Future<void> _accept(HttpRequest request) async {
    // A hub without device keys: `/health` answers without X-Device-Keys,
    // so the client sends its bare Hub token, as before.
    if (request.method == 'GET' && request.uri.path == '/health') {
      await request.response.close();
      return;
    }
    if (request.uri.path == '/auth/redeem') {
      final body = jsonDecode(await utf8.decoder.bind(request).join()) as Map;
      if (body['code'] != 'synthetic-code' || body['device'] != device) {
        request.response.statusCode = HttpStatus.badRequest;
      } else {
        redemptions++;
        request.response.headers.contentType = ContentType.json;
        request.response.write(
          jsonEncode({
            'token': 'synthetic-mobile-token',
            'label': 'Android',
            'account': 'test@example.com',
          }),
        );
      }
      await request.response.close();
      return;
    }
    if (request.uri.path == '/auth/signout' &&
        request.headers.value(HttpHeaders.authorizationHeader) ==
            'Bearer synthetic-mobile-token') {
      signouts++;
      request.response.statusCode = HttpStatus.noContent;
      await request.response.close();
      return;
    }
    if (request.uri.path != '/agent/ws' ||
        request.headers.value(HttpHeaders.authorizationHeader) !=
            'Bearer synthetic-mobile-token' ||
        request.headers.value('X-Device') != device) {
      request.response.statusCode = HttpStatus.unauthorized;
      await request.response.close();
      return;
    }
    final socket = await WebSocketTransformer.upgrade(request);
    sockets.add(socket);
    subscriptions.add(
      socket.listen((dynamic frame) {
        final rpc = jsonDecode(frame as String) as Map<String, dynamic>;
        final method = rpc['method'] as String;
        calls.add(method);
        final params = rpc['params'] as Map<String, dynamic>? ?? {};
        final Object? result;
        switch (method) {
          case 'host_info':
            result = {
              'kind': 'hub',
              'packs': <String>[],
              'tools': ['search_notes', 'read_page', 'update_page', 'js'],
              'providers': ['scripted'],
              'can_run_tasks': false,
              'version': 'test',
              'permission_mode': 'bypass',
              'projects': false,
            };
          case 'models':
            result = [
              {'id': 'scripted/scripted', 'name': 'Test Hub model'},
            ];
          case 'sessions':
            result = sessions;
          case 'create_session':
            final session = {
              'id': 'hub-chat-${sessions.length + 1}',
              'title': params['title'],
              'created_at': '2026-10-11T12:00:00Z',
              'updated_at': '2026-10-11T12:00:00Z',
            };
            sessions.add(session);
            result = session;
          case 'session_state':
            result = {
              'session': sessions.firstWhere(
                (s) => s['id'] == params['session_id'],
              ),
              'messages': <Object>[],
              'runs': <Object>[],
              'pending_approvals': <Object>[],
              'seq': 0,
              'live_seen': 0,
              'streaming_text': '',
            };
          case 'subscribe':
          case 'unsubscribe':
          case 'cancel':
            result = null;
          default:
            socket.add(
              jsonEncode({
                'jsonrpc': '2.0',
                'id': rpc['id'],
                'error': {
                  'code': -32601,
                  'message': 'Unknown test method: $method',
                },
              }),
            );
            return;
        }
        socket.add(
          jsonEncode({'jsonrpc': '2.0', 'id': rpc['id'], 'result': result}),
        );
      }),
    );
  }

  Future<void> close() async {
    for (final socket in sockets) {
      await socket.close();
    }
    for (final sub in subscriptions) {
      await sub.cancel();
    }
    await requests.cancel();
    await server.close(force: true);
  }
}

void main() {
  setUpAll(initCore);

  test(
    'legacy Hub login URL and optional account survive a settings reload',
    () {
      final dir = Directory.systemTemp.createTempSync(
        'mobile-agent-preferences-',
      );
      addTearDown(() => dir.deleteSync(recursive: true));
      final file = File('${dir.path}/settings.json')
        ..writeAsStringSync(
          jsonEncode({
            'clientUrl': 'https://custom-hub.example',
            'clientToken': 'synthetic-mobile-token',
            'clientDeviceLabel': 'Android',
          }),
        );
      final settings = AppSettings.load(file);
      expect(settings.hubSignInUrl, 'https://custom-hub.example');
      expect(settings.clientAccount, isNull);
      settings.update((s) => s.clientAccount = 'test@example.com');
      expect(AppSettings.load(file).clientAccount, 'test@example.com');
      expect(AppSettings.load(file).hubSignInUrl, 'https://custom-hub.example');
    },
  );

  appTest(
    'Android migrates the old agent-off setting and never opens locally',
    (tester) async {
      final tv = await pumpApp(tester, mobile: true);
      final file = File('${tv.dir.path}/legacy-ai.json')
        ..writeAsStringSync('{"agentEnabled":false,"agentHost":"local"}');
      final settings = AiSettings.load(file);
      final store = AgentStore.attach(tv.vault, settings)
        ..useAppSettings(tv.app.settings);
      expect(defaultTargetPlatform, TargetPlatform.android);
      expect(localAgentPlatform, isFalse);
      expect(settings.agentEnabled, isTrue);
      expect(AgentStore.canRun, isTrue);
      expect(store.enabled, isTrue);
      expect(store.host, AgentHost.hub);
      expect(store.requiresHubSignIn, isTrue);
      expect(await real(tester, store.ensureOpen), isNull);
      expect(store.openError, isNull);
      expect(store.isOpen, isFalse);
    },
  );

  appTest('Android signed-in New chat uses its Hub token and vault device ID', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: true);
    final hub = await real(tester, () => _StubHub.start(tv.vault.deviceId));
    final store = AgentStore.of(tv.app);
    final links = StreamController<Uri>.broadcast(sync: true);
    final auth = HubAuthService(
      android: true,
      androidLinks: links.stream,
      launchBrowser: (start) async {
        expect(start.queryParameters['redirect'], 'notes://auth');
        links.add(
          Uri.parse('notes://auth').replace(
            queryParameters: {
              'code': 'synthetic-code',
              'state': start.queryParameters['state']!,
            },
          ),
        );
        return true;
      },
    );
    try {
      await real(
        tester,
        () => _localHttp(
          () => signInToHub(tv.app.sync, tv.app.settings, auth, hub.url),
        ),
      );
      expect(hub.redemptions, 1);
      expect(tv.app.settings.clientAccount, 'test@example.com');
      expect(tv.app.settings.clientDeviceLabel, 'Android');
      expect(tv.app.sync.connected, isTrue);
      expect(tv.app.sync.hubUrl, hub.url);
      expect(store.host, AgentHost.hub);
      expect(store.requiresHubSignIn, isFalse);
      unawaited(openAgent(tester.element(find.byType(HomeScreen))));
      await settleUntil(tester, () => store.isOpen && store.models.isNotEmpty);
      expect(store.openError, isNull);
      expect(key('agent-sign-in-prompt'), findsNothing);
      expect(find.byType(AgentComposer), findsOneWidget);
      expect(
        tester.widget<IconBtn>(key('agent-new-chat-mobile')).onTap,
        isNotNull,
      );

      await tapAndSettle(tester, key('agent-new-chat-mobile'));
      await settleUntil(tester, () => store.openId == 'hub-chat-1');
      expect(hub.calls, contains('create_session'));
      expect(store.state?.session.title, 'New chat');
      expect(store.models.single.id, 'scripted/scripted');

      // This transport stub refuses sync. The phone must surface that failure
      // before requesting a model turn against a potentially stale replica.
      await real(tester, () => store.send('Read my notes'));
      expect(store.error, isNotNull);
      expect(hub.calls, isNot(contains('send')));

      // Clearing login immediately prevents further commands, even before
      // the existing debounce has finished closing the remote connection.
      await real(
        tester,
        () => _localHttp(
          () => signOutFromHub(tv.app.sync, tv.app.settings, auth),
        ),
      );
      expect(hub.signouts, 1);
      expect(tv.app.settings.clientToken, isNull);
      expect(tv.app.settings.clientAccount, isNull);
      expect(tv.app.sync.connected, isFalse);
      expect(store.openId, isNull);
      expect(store.sessions, isEmpty);
      expect(store.state, isNull);
      expect(await real(tester, store.ensureOpen), isNull);
      await settle(tester);
      expect(store.host, AgentHost.hub);
      expect(key('agent-sign-in-prompt'), findsOneWidget);
      expect(find.byType(AgentComposer), findsNothing);
      expect(
        tester.widget<IconBtn>(key('agent-new-chat-mobile')).onTap,
        isNull,
      );
      expect(tester.takeException(), isNull);
    } finally {
      auth.dispose();
      await links.close();
      await real(tester, tv.app.sync.disconnect);
      await real(tester, store.close);
      await real(tester, hub.close);
    }
  });
}
