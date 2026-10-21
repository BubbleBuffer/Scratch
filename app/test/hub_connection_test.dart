import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/features/sync/hub_auth.dart';
import 'package:notes_app/features/sync/sync_control.dart';
import 'package:notes_app/state/sync_store.dart';

import 'harness.dart' show initCore;

class _Vault implements Vault {
  bool _closed = false;
  final _closeListeners = <VoidCallback>[];

  @override
  bool get isClosed => _closed;

  @override
  void addCloseListener(VoidCallback listener) {
    if (_closed) {
      listener();
    } else {
      _closeListeners.add(listener);
    }
  }

  @override
  void removeCloseListener(VoidCallback listener) =>
      _closeListeners.remove(listener);

  @override
  void dispose() {
    if (_closed) return;
    _closed = true;
    for (final listener in List.of(_closeListeners)) {
      listener();
    }
    _closeListeners.clear();
  }

  @override
  String get deviceId => 'existing-vault-device';
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Sync extends SyncStore {
  _Sync() : super(_Vault());
  String? token;
  bool running = false;
  bool disconnected = false;
  Object? connectError;

  @override
  bool get connected => running;
  @override
  Future<void> connect(String url, String token, {SyncOptions? options, List<String> trusted = const []}) async {
    if (connectError case final error?) throw error;
    hubUrl = url;
    this.token = token;
    running = true;
  }

  @override
  Future<void> disconnect() async {
    running = false;
    hubUrl = null;
    token = null;
    disconnected = true;
  }
}

void main() {
  setUpAll(initCore);

  test('failed LAN pairing leaves saved credentials and label unchanged', () async {
    final dir = Directory.systemTemp.createTempSync('notes-hub-connect-failure-');
    addTearDown(() => dir.deleteSync(recursive: true));
    final settings = AppSettings(file: File('${dir.path}/settings.json'));
    settings.update((s) {
      s.clientUrl = 'https://previous.example';
      s.clientToken = 'previous-device-token';
      s.clientDeviceLabel = 'Previous device';
    });
    final original = settings.file!.readAsStringSync();
    var notifications = 0;
    settings.addListener(() => notifications++);
    final sync = _Sync()..connectError = StateError('Connection failed');
    addTearDown(sync.dispose);

    await expectLater(
      connectToHub(sync, settings, 'http://bad-host:47100', 'bad-device-token'),
      throwsA(isA<StateError>()),
    );

    expect(settings.clientUrl, 'https://previous.example');
    expect(settings.clientToken, 'previous-device-token');
    expect(settings.clientDeviceLabel, 'Previous device');
    expect(settings.file!.readAsStringSync(), original);
    expect(notifications, 0);
    expect(sync.connected, isFalse);
    final restored = AppSettings.load(settings.file!);
    expect(restored.clientUrl, settings.clientUrl);
    expect(restored.clientToken, settings.clientToken);
    expect(restored.clientDeviceLabel, settings.clientDeviceLabel);
  });

  test(
    'desktop browser loopback redeems, saves and starts the paired connection',
    () async {
      final dir = Directory.systemTemp.createTempSync('notes-hub-connection-');
      final settings = AppSettings(file: File('${dir.path}/settings.json'));
      final sync = _Sync();
      addTearDown(sync.dispose);
      final hub = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      final hubUrl = 'http://127.0.0.1:${hub.port}';
      final browser = HttpClient();
      String? label;
      final redeemed = Completer<void>();
      final requests = hub.listen((request) async {
        if (request.uri.path == '/auth/start') {
          expect(request.uri.queryParameters['device'], sync.vault.deviceId);
          label = request.uri.queryParameters['name'];
          expect(label, isNotEmpty);
          final redirect = Uri.parse(request.uri.queryParameters['redirect']!);
          expect(redirect.host, '127.0.0.1');
          expect(redirect.path, '/done');
          expect(redirect.port, inInclusiveRange(1024, 65535));
          request.response.statusCode = HttpStatus.found;
          request.response.headers.set(
            HttpHeaders.locationHeader,
            redirect
                .replace(
                  queryParameters: {
                    'code': 'one-use-test-code',
                    'state': request.uri.queryParameters['state']!,
                  },
                )
                .toString(),
          );
        } else if (request.uri.path == '/auth/redeem') {
          expect(request.method, 'POST');
          expect(jsonDecode(await utf8.decoder.bind(request).join()), {
            'code': 'one-use-test-code',
            'device': sync.vault.deviceId,
          });
          request.response.headers.contentType = ContentType.json;
          request.response.write(
            jsonEncode({'token': 'test-device-token', 'label': label}),
          );
          redeemed.complete();
        } else {
          request.response.statusCode = HttpStatus.notFound;
        }
        await request.response.close();
      });
      final auth = HubAuthService(
        android: false,
        launchBrowser: (uri) async {
          final response = await (await browser.getUrl(uri)).close();
          expect(response.statusCode, HttpStatus.ok);
          expect(
            await utf8.decoder.bind(response).join(),
            contains('Signed in, you can close this tab'),
          );
          return true;
        },
      );
      try {
        await signInToHub(sync, settings, auth, '$hubUrl/');
        await redeemed.future;
        final restored = AppSettings.load(settings.file!);
        expect(restored.clientUrl, hubUrl);
        expect(restored.clientToken, 'test-device-token');
        expect(restored.clientDeviceLabel, label);
        expect(sync.connected, isTrue);
        expect(sync.hubUrl, hubUrl);
        expect(sync.token, restored.clientToken);
        expect(sync.vault.deviceId, 'existing-vault-device');
      } finally {
        auth.dispose();
        browser.close(force: true);
        await requests.cancel();
        await hub.close(force: true);
        dir.deleteSync(recursive: true);
      }
    },
  );

  for (final status in [HttpStatus.noContent, HttpStatus.serviceUnavailable]) {
    test(
      'sign out calls the hub and clears sync and Hub agent choice after HTTP $status',
      () async {
        final dir = Directory.systemTemp.createTempSync('notes-hub-signout-');
        final settings = AppSettings(file: File('${dir.path}/settings.json'));
        final hub = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
        final hubUrl = 'http://127.0.0.1:${hub.port}';
        final requestSeen = Completer<void>();
        final requests = hub.listen((request) async {
          expect(request.method, 'POST');
          expect(request.uri.path, '/auth/signout');
          expect(
            request.headers.value(HttpHeaders.authorizationHeader),
            'Bearer test-device-token',
          );
          request.response.statusCode = status;
          await request.response.close();
          requestSeen.complete();
        });
        final sync = _Sync();
        addTearDown(sync.dispose);
        final auth = HubAuthService(android: false);
        try {
          await connectToHub(
            sync,
            settings,
            hubUrl,
            'test-device-token',
            deviceLabel: 'Desktop',
          );
          AiSettings.beside(settings.file)
              .update((s) => s.agentHost = AgentHost.hub);
          final signout = signOutFromHub(sync, settings, auth);
          if (status == HttpStatus.noContent) {
            await signout;
          } else {
            await expectLater(signout, throwsA(isA<HubAuthException>()));
          }
          await requestSeen.future;
          final restored = AppSettings.load(settings.file!);
          expect(restored.clientUrl, isNull);
          expect(restored.clientToken, isNull);
          expect(restored.clientDeviceLabel, isNull);
          expect(AiSettings.beside(settings.file).agentHost, AgentHost.local);
          expect(sync.connected, isFalse);
          expect(sync.disconnected, isTrue);
        } finally {
          auth.dispose();
          await requests.cancel();
          await hub.close(force: true);
          dir.deleteSync(recursive: true);
        }
      },
    );
  }
}
