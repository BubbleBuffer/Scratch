import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/sync/hub_auth.dart';
import 'package:notes_app/features/sync/sync_settings.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';
import 'sync_helpers.dart';

class _BrowserLogin extends HubAuthService {
  _BrowserLogin() : super(android: false);
  Completer<HubAuthResult>? pending;
  String? requestedUrl;
  String? requestedDevice;
  String? revokedToken;

  @override
  Future<HubAuthResult> signIn({
    required String hubUrl,
    required String deviceId,
    required String deviceName,
  }) {
    requestedUrl = hubUrl;
    requestedDevice = deviceId;
    pending = Completer<HubAuthResult>();
    return pending!.future;
  }

  @override
  void cancel() {
    if (pending != null && !pending!.isCompleted) {
      pending!.completeError(const HubAuthException('Sign in cancelled.'));
    }
  }

  @override
  Future<void> signOut({required String hubUrl, required String token}) async {
    revokedToken = token;
  }
}

class _HubPage extends StatelessWidget {
  const _HubPage(this.auth);
  final HubAuthService auth;

  @override
  Widget build(BuildContext context) => ColoredBox(
    color: context.colors.bgPrimary,
    child: SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: HubSignInSettings(auth: auth),
    ),
  );
}

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    appTest('Hub sign-in controls on ${mobile ? 'Android' : 'desktop'}', (
      tester,
    ) async {
      final tv = await pumpApp(tester, mobile: mobile);
      final auth = _BrowserLogin();
      Navigator.of(
        tester.element(find.byType(HomeScreen)),
      ).push(PageRouteBuilder<void>(pageBuilder: (_, _, _) => _HubPage(auth)));
      await settle(tester);
      // No built-in hub: the field starts empty and the user enters theirs.
      expect(
        tester.widget<EditableText>(
          find.descendant(
            of: key('hub-sign-in-url'),
            matching: find.byType(EditableText),
          ),
        ).controller.text,
        isEmpty,
      );
      await tester.enterText(
        find.descendant(
          of: key('hub-sign-in-url'),
          matching: find.byType(EditableText),
        ),
        'https://notes.example.com',
      );
      await settle(tester);
      expect(key('hub-sign-in'), findsOneWidget);

      await tapAndSettle(tester, key('hub-sign-in'));
      expect(auth.requestedUrl, 'https://notes.example.com');
      expect(auth.requestedDevice, tv.vault.deviceId);
      expect(find.text('Waiting for the browser…'), findsOneWidget);
      expect(key('hub-sign-in-cancel'), findsOneWidget);
      await tapAndSettle(tester, key('hub-sign-in-cancel'));
      expect(key('hub-sign-in'), findsOneWidget);
      expect(key('hub-sign-in-error'), findsNothing);

      await tapAndSettle(tester, key('hub-sign-in'));
      auth.pending!.completeError(
        const HubAuthException('Sign in timed out. Please try again.'),
      );
      await settle(tester);
      expect(find.text('Sign in timed out. Please try again.'), findsOneWidget);
      expect(key('hub-sign-in'), findsOneWidget);

      await tapAndSettle(tester, key('hub-sign-in'));
      auth.pending!.complete(
        HubAuthResult(
          hubUrl: 'http://127.0.0.1:1',
          token: 'test-device-token',
          deviceId: tv.vault.deviceId,
          deviceName: mobile ? 'Android' : 'Desktop computer',
        ),
      );
      await settleUntil(
        tester,
        () => key('hub-sign-out').evaluate().isNotEmpty,
      );
      expect(
        find.text(
          'Signed in to 127.0.0.1 as ${mobile ? 'Android' : 'Desktop computer'}',
        ),
        findsOneWidget,
      );
      expect(tv.app.settings.clientToken, 'test-device-token');
      expect(tv.app.sync.connected, isTrue);
      expect(key('hub-sign-in-error'), findsNothing);

      await tapAndSettle(tester, key('hub-sign-out'));
      expect(auth.revokedToken, 'test-device-token');
      expect(tv.app.settings.clientToken, isNull);
      expect(tv.app.sync.connected, isFalse);
      expect(key('hub-sign-in'), findsOneWidget);
      expect(tester.takeException(), isNull);
    });
  }
}
