import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/sync/hub_auth.dart';

void main() {
  late _FakeHub hub;
  setUp(() async => hub = await _FakeHub.start());
  tearDown(() async => hub.close());

  test('desktop browser loopback redeems the code for this device', () async {
    final auth = HubAuthService(android: false, launchBrowser: _browse);
    addTearDown(auth.dispose);
    final result = await auth.signIn(
      hubUrl: '${hub.url}/',
      deviceId: 'vault-device',
      deviceName: 'My PC',
    );
    expect(result.hubUrl, hub.url);
    expect(result.token, 'synthetic-device-token');
    expect(result.deviceId, 'vault-device');
    expect(result.deviceName, 'My PC (2)');
    expect(hub.redeemedDevice, 'vault-device');
    expect(hub.redeemedCode, 'synthetic-code');
    expect(hub.redirect?.host, '127.0.0.1');
    expect(hub.redirect?.path, '/done');
    expect(hub.redirect!.port, inInclusiveRange(1024, 65535));
    expect(hub.browserPage, contains('Signed in, you can close this tab'));
    await _expectClosed(hub.redirect!);
  });

  test('cancel closes the loopback listener and never redeems', () async {
    final opened = Completer<Uri>();
    final auth = HubAuthService(
      android: false,
      launchBrowser: (uri) async {
        opened.complete(uri);
        return true;
      },
    );
    addTearDown(auth.dispose);
    final login = auth.signIn(
      hubUrl: hub.url,
      deviceId: 'device',
      deviceName: 'PC',
    );
    final expectation = expectLater(
      login,
      throwsA(
        isA<HubAuthException>().having(
          (e) => e.message,
          'message',
          'Sign in cancelled.',
        ),
      ),
    );
    final start = await opened.future;
    final redirect = Uri.parse(start.queryParameters['redirect']!);
    auth.cancel();
    await expectation;
    await _expectClosed(redirect);
    expect(hub.redeemedCode, isNull);
  });

  test('timeout closes the loopback listener and never redeems', () async {
    final opened = Completer<Uri>();
    final auth = HubAuthService(
      android: false,
      timeout: const Duration(milliseconds: 80),
      launchBrowser: (uri) async {
        opened.complete(uri);
        return true;
      },
    );
    addTearDown(auth.dispose);
    final login = auth.signIn(
      hubUrl: hub.url,
      deviceId: 'device',
      deviceName: 'PC',
    );
    final expectation = expectLater(
      login,
      throwsA(
        isA<HubAuthException>().having(
          (e) => e.message,
          'message',
          contains('timed out'),
        ),
      ),
    );
    final start = await opened.future;
    await expectation;
    await _expectClosed(Uri.parse(start.queryParameters['redirect']!));
    expect(hub.redeemedCode, isNull);
  });

  test(
    'Android subscribes before launch and redeems a matching deep link',
    () async {
      final links = StreamController<Uri>.broadcast(sync: true);
      addTearDown(links.close);
      late Uri start;
      final auth = HubAuthService(
        android: true,
        androidLinks: links.stream,
        launchBrowser: (uri) async {
          start = uri;
          expect(links.hasListener, isTrue);
          links.add(
            Uri.parse('notes://auth?code=wrong-code&state=wrong-state'),
          );
          links.add(
            Uri.parse('notes://auth').replace(
              queryParameters: {
                'code': 'synthetic-code',
                'state': uri.queryParameters['state']!,
              },
            ),
          );
          return true;
        },
      );
      addTearDown(auth.dispose);
      final result = await auth.signIn(
        hubUrl: hub.url,
        deviceId: 'android-device',
        deviceName: 'Android',
      );
      expect(start.queryParameters['redirect'], 'notes://auth');
      expect(result.token, 'synthetic-device-token');
      expect(hub.redeemedCode, 'synthetic-code');
      expect(hub.redeemedDevice, 'android-device');
      expect(links.hasListener, isFalse);
    },
  );

  test('Android callback parser ignores other destinations, states and duplicate codes', () {
    String? parse(String uri) =>
        hubAuthCallbackCode(Uri.parse(uri), state: 'expected', android: true);
    expect(parse('notes://auth?code=one&state=expected'), 'one');
    for (final uri in [
      'notes://pair?code=one&state=expected',
      'https://auth?code=one&state=expected',
      'notes://auth/path?code=one&state=expected',
      'notes://auth:1234?code=one&state=expected',
      'notes://auth?code=one&state=wrong',
      'notes://auth?code=one&code=two&state=expected',
      'notes://auth?code=one&state=expected&state=expected',
      'notes://auth?code=&state=expected',
    ]) {
      expect(parse(uri), isNull, reason: uri);
    }
  });

  test('Android cancellation unsubscribes and ignores a late return', () async {
    final links = StreamController<Uri>.broadcast(sync: true);
    addTearDown(links.close);
    final opened = Completer<Uri>();
    final auth = HubAuthService(
      android: true,
      androidLinks: links.stream,
      launchBrowser: (uri) async {
        opened.complete(uri);
        return true;
      },
    );
    addTearDown(auth.dispose);
    final login = auth.signIn(
      hubUrl: hub.url,
      deviceId: 'device',
      deviceName: 'Android',
    );
    final expectation = expectLater(login, throwsA(isA<HubAuthException>()));
    final start = await opened.future;
    auth.cancel();
    await expectation;
    expect(links.hasListener, isFalse);
    links.add(
      Uri.parse('notes://auth').replace(
        queryParameters: {
          'code': 'synthetic-code',
          'state': start.queryParameters['state']!,
        },
      ),
    );
    expect(hub.redeemedCode, isNull);
  });

  test('callback refusal never exposes hub error text', () async {
    final links = StreamController<Uri>.broadcast(sync: true);
    addTearDown(links.close);
    final auth = HubAuthService(
      android: true,
      androidLinks: links.stream,
      launchBrowser: (uri) async {
        links.add(
          Uri.parse('notes://auth').replace(
            queryParameters: {
              'error': 'private-provider-error',
              'state': uri.queryParameters['state']!,
            },
          ),
        );
        return true;
      },
    );
    addTearDown(auth.dispose);
    await expectLater(
      auth.signIn(hubUrl: hub.url, deviceId: 'device', deviceName: 'Android'),
      throwsA(
        isA<HubAuthException>().having(
          (e) => e.message,
          'message',
          'Sign in was refused. Please try again.',
        ),
      ),
    );
    expect(hub.redeemedCode, isNull);
  });

  test('redeem failure is plain and a new login may follow', () async {
    hub.redeemStatus = HttpStatus.badRequest;
    final auth = HubAuthService(android: false, launchBrowser: _browse);
    addTearDown(auth.dispose);
    await expectLater(
      auth.signIn(hubUrl: hub.url, deviceId: 'device', deviceName: 'PC'),
      throwsA(
        isA<HubAuthException>().having(
          (e) => e.message,
          'message',
          'Could not finish sign in. Please try again.',
        ),
      ),
    );
    hub.redeemStatus = HttpStatus.ok;
    final result = await auth.signIn(
      hubUrl: hub.url,
      deviceId: 'device',
      deviceName: 'PC',
    );
    expect(result.token, 'synthetic-device-token');
  });

  test('sign out posts the token in the Authorization header', () async {
    final auth = HubAuthService();
    addTearDown(auth.dispose);
    await auth.signOut(hubUrl: hub.url, token: 'synthetic-device-token');
    expect(hub.signoutMethod, 'POST');
    expect(hub.signoutAuthorization, 'Bearer synthetic-device-token');
  });

  test(
    'cancel during redemption never delivers credentials for storage',
    () async {
      hub.holdRedeem = Completer<void>();
      final auth = HubAuthService(android: false, launchBrowser: _browse);
      addTearDown(auth.dispose);
      var delivered = false;
      final login = auth
          .signIn(hubUrl: hub.url, deviceId: 'device', deviceName: 'PC')
          .then((result) {
            delivered = true;
            return result;
          });
      final expectation = expectLater(login, throwsA(isA<HubAuthException>()));
      await hub.redeemStarted.future;
      auth.cancel();
      await expectation;
      hub.holdRedeem!.complete();
      await Future<void>.delayed(const Duration(milliseconds: 20));
      expect(delivered, isFalse);
    },
  );

  test('invalid hub URL fails before opening a browser', () async {
    final auth = HubAuthService(
      launchBrowser: (_) async => fail('Browser must not open'),
    );
    addTearDown(auth.dispose);
    await expectLater(
      auth.signIn(
        hubUrl: 'https://user:private@hub/',
        deviceId: 'device',
        deviceName: 'PC',
      ),
      throwsA(isA<HubAuthException>()),
    );
  });
}

Future<bool> _browse(Uri start) async {
  final client = HttpClient();
  try {
    final request = await client.getUrl(start);
    final response = await request.close();
    final page = await utf8.decoder.bind(response).join();
    _FakeHub.last!.browserPage = page;
    return response.statusCode == HttpStatus.ok;
  } finally {
    client.close(force: true);
  }
}

Future<void> _expectClosed(Uri redirect) async {
  await expectLater(
    Socket.connect(
      redirect.host,
      redirect.port,
      timeout: const Duration(seconds: 1),
    ),
    throwsA(isA<SocketException>()),
  );
}

class _FakeHub {
  _FakeHub(this.server);
  static _FakeHub? last;
  final HttpServer server;
  String get url => 'http://127.0.0.1:${server.port}';
  Uri? redirect;
  String? redeemedCode;
  String? redeemedDevice;
  String? browserPage;
  String? signoutAuthorization;
  String? signoutMethod;
  int redeemStatus = HttpStatus.ok;
  final redeemStarted = Completer<void>();
  Completer<void>? holdRedeem;

  static Future<_FakeHub> start() async {
    final hub = _FakeHub(
      await HttpServer.bind(InternetAddress.loopbackIPv4, 0),
    );
    last = hub;
    hub.server.listen(hub.handle);
    return hub;
  }

  Future<void> handle(HttpRequest request) async {
    switch (request.uri.path) {
      case '/auth/start':
        redirect = Uri.parse(request.uri.queryParameters['redirect']!);
        request.response.statusCode = HttpStatus.found;
        request.response.headers.set(
          HttpHeaders.locationHeader,
          redirect!
              .replace(
                queryParameters: {
                  'code': 'synthetic-code',
                  'state': request.uri.queryParameters['state']!,
                },
              )
              .toString(),
        );
      case '/auth/redeem':
        final body = jsonDecode(
          await utf8.decoder.bind(request).join(),
        ) as Map<String, dynamic>;
        redeemedCode = body['code'];
        redeemedDevice = body['device'];
        if (!redeemStarted.isCompleted) redeemStarted.complete();
        await holdRedeem?.future;
        request.response.statusCode = redeemStatus;
        request.response.headers.contentType = ContentType.json;
        request.response.write(
          jsonEncode({'token': 'synthetic-device-token', 'label': 'My PC (2)'}),
        );
      case '/auth/signout':
        signoutMethod = request.method;
        signoutAuthorization = request.headers.value(
          HttpHeaders.authorizationHeader,
        );
        request.response.statusCode = HttpStatus.noContent;
      default:
        request.response.statusCode = HttpStatus.notFound;
    }
    try {
      await request.response.close();
    } catch (_) {
      // Cancellation deliberately closes the client's pending redeem socket.
    }
  }

  Future<void> close() => server.close(force: true);
}
