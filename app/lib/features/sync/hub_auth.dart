import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:app_links/app_links.dart';
import 'package:url_launcher/url_launcher.dart';

import '../../ui/app_name.dart';

typedef HubBrowserLauncher = Future<bool> Function(Uri url);

class HubAuthException implements Exception {
  const HubAuthException(this.message);
  final String message;
  @override
  String toString() => message;
}

class HubAuthResult {
  const HubAuthResult({
    required this.hubUrl,
    required this.token,
    required this.deviceId,
    required this.deviceName,
    this.account,
  });
  final String hubUrl;
  final String token;
  final String deviceId;
  final String deviceName;
  final String? account;
}

/// Returns a code only for this login's callback. Unrelated links are ignored.
/// Never include the callback URI or the hub's response in an error message.
String? hubAuthCallbackCode(
  Uri uri, {
  required String state,
  required bool android,
}) {
  final destination = android
      ? uri.scheme == 'notes' &&
            uri.host == 'auth' &&
            uri.path.isEmpty &&
            !uri.hasPort
      : uri.path == '/done';
  final query = uri.queryParametersAll;
  if (!destination ||
      uri.userInfo.isNotEmpty ||
      uri.hasFragment ||
      query['state']?.length != 1 ||
      query['state']?.single != state) {
    return null;
  }
  if (query.containsKey('error')) {
    throw const HubAuthException('Sign in was refused. Please try again.');
  }
  final codes = query['code'];
  if (codes == null || codes.length != 1 || codes.single.isEmpty) return null;
  return codes.single;
}

/// Browser login independent of the vault bridge. The caller stores the result
/// using the existing sync connection settings after redemption succeeds.
class HubAuthService {
  HubAuthService({
    HubBrowserLauncher? launchBrowser,
    this.androidLinks,
    bool? android,
    this.timeout = const Duration(minutes: 5),
  }) : _launchBrowser = launchBrowser ?? _openBrowser,
       _android = android ?? Platform.isAndroid;

  final HubBrowserLauncher _launchBrowser;
  final Stream<Uri>? androidLinks;
  final bool _android;
  final Duration timeout;
  _LoginAttempt? _attempt;
  bool _disposed = false;

  static Future<bool> _openBrowser(Uri uri) =>
      launchUrl(uri, mode: LaunchMode.externalApplication);

  Future<HubAuthResult> signIn({
    required String hubUrl,
    required String deviceId,
    required String deviceName,
  }) async {
    if (_disposed) throw const HubAuthException('Sign in is unavailable.');
    if (_attempt != null) {
      throw const HubAuthException('Sign in is already in progress.');
    }
    final hub = _hubUri(hubUrl);
    final attempt = _LoginAttempt();
    _attempt = attempt;
    attempt.timer = Timer(
      timeout,
      () => attempt.fail(
        const HubAuthException('Sign in timed out. Please try again.'),
      ),
    );
    try {
      return await Future.any([
        _signIn(attempt, hub, deviceId, deviceName),
        attempt.failure.future,
      ]);
    } on HubAuthException {
      rethrow;
    } catch (_) {
      throw const HubAuthException(
        'Could not sign in to the hub. Please try again.',
      );
    } finally {
      attempt.active = false;
      attempt.timer?.cancel();
      await attempt.links?.cancel();
      await attempt.server?.close(force: true);
      attempt.client.close(force: true);
      if (identical(_attempt, attempt)) _attempt = null;
    }
  }

  Future<HubAuthResult> _signIn(
    _LoginAttempt attempt,
    Uri hub,
    String device,
    String name,
  ) async {
    final random = Random.secure();
    final state = base64UrlEncode(
      List.generate(32, (_) => random.nextInt(256)),
    );
    final callback = Completer<String>();
    void receive(Uri uri) {
      if (!attempt.active || callback.isCompleted) return;
      try {
        final code = hubAuthCallbackCode(uri, state: state, android: _android);
        if (code != null) callback.complete(code);
      } on HubAuthException catch (e) {
        attempt.fail(e);
      }
    }

    late final Uri redirect;
    if (_android) {
      // Subscribe before opening the browser so a quick return is not lost.
      attempt.links = (androidLinks ?? AppLinks().uriLinkStream).listen(
        receive,
        onError: (_) => attempt.fail(
          const HubAuthException('Could not receive the browser sign in.'),
        ),
      );
      redirect = Uri.parse('notes://auth');
    } else {
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      if (!attempt.active) {
        await server.close(force: true);
        throw const HubAuthException('Sign in cancelled.');
      }
      attempt.server = server;
      redirect = Uri.parse('http://127.0.0.1:${server.port}/done');
      server.listen(
        (request) async {
          final valid =
              request.method == 'GET' &&
              request.uri.path == '/done' &&
              request.uri.queryParametersAll['state']?.length == 1 &&
              request.uri.queryParameters['state'] == state;
          final success =
              valid &&
              request.uri.queryParametersAll['code']?.length == 1 &&
              (request.uri.queryParameters['code']?.isNotEmpty ?? false) &&
              !request.uri.queryParameters.containsKey('error');
          try {
            request.response.headers
              ..contentType = ContentType.html
              ..set(HttpHeaders.cacheControlHeader, 'no-store')
              ..set('Referrer-Policy', 'no-referrer');
            request.response.statusCode = valid
                ? HttpStatus.ok
                : HttpStatus.badRequest;
            request.response.write(
              success
                  ? '<!doctype html><title>$appName</title><p>Signed in, you can close this tab.</p>'
                  : '<!doctype html><title>$appName</title><p>Return to $appName to try signing in again.</p>',
            );
            await request.response.close();
          } catch (_) {
            // A browser closing this tab must not cancel redemption.
          }
          if (valid) receive(request.uri);
        },
        onError: (_) => attempt.fail(
          const HubAuthException('Could not receive the browser sign in.'),
        ),
      );
    }
    final start = hub
        .resolve('/auth/start')
        .replace(
          queryParameters: {
            'device': device,
            'name': name,
            'redirect': redirect.toString(),
            'state': state,
          },
        );
    if (!await _launchBrowser(start)) {
      throw const HubAuthException(
        'Could not open the browser. Please try again.',
      );
    }
    if (!attempt.active) throw const HubAuthException('Sign in cancelled.');
    final code = await callback.future;
    if (!attempt.active) throw const HubAuthException('Sign in cancelled.');
    final request = await attempt.client.postUrl(hub.resolve('/auth/redeem'));
    request.followRedirects = false;
    request.headers.contentType = ContentType.json;
    request.write(jsonEncode({'code': code, 'device': device}));
    final response = await request.close();
    if (response.statusCode != HttpStatus.ok) {
      throw const HubAuthException(
        'Could not finish sign in. Please try again.',
      );
    }
    final body = jsonDecode(await utf8.decoder.bind(response).join());
    if (body is! Map<String, dynamic> ||
        body['token'] is! String ||
        (body['token'] as String).isEmpty ||
        body['label'] is! String ||
        (body['label'] as String).isEmpty) {
      throw const HubAuthException(
        'The hub returned an invalid sign in response.',
      );
    }
    if (!attempt.active) throw const HubAuthException('Sign in cancelled.');
    return HubAuthResult(
      hubUrl: hub.toString(),
      token: body['token'],
      deviceId: device,
      deviceName: body['label'],
      account:
          body['account'] is String &&
              (body['account'] as String).trim().isNotEmpty
          ? body['account'] as String
          : null,
    );
  }

  void cancel() => _attempt?.fail(const HubAuthException('Sign in cancelled.'));

  void dispose() {
    _disposed = true;
    cancel();
  }

  Future<void> signOut({required String hubUrl, required String token}) async {
    final client = HttpClient()
      ..connectionTimeout = const Duration(seconds: 15);
    try {
      await (() async {
        final request = await client.postUrl(
          _hubUri(hubUrl).resolve('/auth/signout'),
        );
        request.followRedirects = false;
        request.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
        final response = await request.close();
        await response.drain<void>();
        if (response.statusCode != HttpStatus.noContent &&
            response.statusCode != HttpStatus.unauthorized) {
          throw const HubAuthException('Could not sign out on the hub.');
        }
      })().timeout(const Duration(seconds: 15));
    } catch (_) {
      throw const HubAuthException('Could not sign out on the hub.');
    } finally {
      client.close(force: true);
    }
  }
}

Uri _hubUri(String value) {
  final uri = Uri.tryParse(value.trim());
  // Plain HTTP loopback also permits testing against a local fake hub.
  final loopback =
      uri?.scheme == 'http' &&
      (uri?.host == '127.0.0.1' || uri?.host == 'localhost');
  if (uri == null ||
      (uri.scheme != 'https' && !loopback) ||
      uri.host.isEmpty ||
      uri.userInfo.isNotEmpty ||
      uri.hasQuery ||
      uri.hasFragment ||
      (uri.path.isNotEmpty && uri.path != '/')) {
    throw const HubAuthException(
      'Enter a hub URL such as https://notes.breadcat.cc.',
    );
  }
  return uri.replace(path: '');
}

class _LoginAttempt {
  bool active = true;
  final failure = Completer<HubAuthResult>();
  final client = HttpClient()..connectionTimeout = const Duration(seconds: 15);
  HttpServer? server;
  StreamSubscription<Uri>? links;
  Timer? timer;
  void fail(HubAuthException error) {
    if (!active) return;
    active = false;
    failure.completeError(error);
  }
}
