// Android (platform override, phone size): Settings > AI > Agent is the Hub's
// sign-in, connection status and default model; a signed-out Agent chat
// offers Sign in instead of "The agent is off".
//
// No Hub is contacted: the login is a fake, the signed-in state is set on
// the settings directly (the agent is never opened) and the models are stubs.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/sync/hub_auth.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';
import 'sync_helpers.dart';

const _stubModels = [
  AgentModel(id: 'hub/fable', name: 'Fable 5.1'),
  AgentModel(id: 'hub/opus', name: 'Opus 5.5'),
];

class _FakeLogin extends HubAuthService {
  _FakeLogin() : super(android: false);
  Completer<HubAuthResult>? pending;
  String? requestedUrl;
  String? revokedToken;

  @override
  Future<HubAuthResult> signIn({
    required String hubUrl,
    required String deviceId,
    required String deviceName,
  }) {
    requestedUrl = hubUrl;
    pending = Completer<HubAuthResult>();
    return pending!.future;
  }

  @override
  Future<void> signOut({required String hubUrl, required String token}) async {
    revokedToken = token;
  }
}

/// A full-screen page with the Hub section (fake login), as Settings shows it.
class _HubAgentPage extends StatelessWidget {
  const _HubAgentPage(this.store, this.auth);
  final AgentStore store;
  final HubAuthService auth;

  @override
  Widget build(BuildContext context) => ColoredBox(
    color: context.colors.bgPrimary,
    child: SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: MobileAgentSettings(store: store, auth: auth),
    ),
  );
}

/// Fake signed-in state on the device (no connection is made).
void signInFixture(TestVault tv, {String? account}) {
  tv.app.settings.update((s) {
    s.clientUrl = 'https://hub.test';
    s.clientToken = 'test-device-token';
    s.clientDeviceLabel = 'Android';
    s.clientAccount = account;
  });
}

Future<void> openAiSettings(WidgetTester tester) async {
  await openMobileSettings(tester);
  await tapAndSettle(tester, key('settings-nav-ai'));
}

void main() {
  setUpAll(initCore);

  appTest('Android Settings > AI shows the Hub agent section signed out', (
    tester,
  ) async {
    await pumpApp(tester, mobile: true);
    await openAiSettings(tester);

    expect(find.text('Agent'), findsOneWidget);
    expect(find.text('https://notes.breadcat.cc'), findsOneWidget);
    expect(key('hub-sign-in'), findsOneWidget);
    expect(
      find.text('Not connected. Sign in to the Hub to use the agent.'),
      findsOneWidget,
    );
    expect(key('agent-default-model'), findsNothing);
    expect(key('agent-host-info'), findsNothing);
    // Desktop-only controls stay off the phone.
    expect(key('ai-runner'), findsNothing);
    expect(key('agent-enabled-toggle'), findsNothing);
    expect(key('ai-mcp-toggle'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  appTest('the Hub address is editable and saved on the device', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: true);
    await openAiSettings(tester);

    await enterField(tester, 'hub-sign-in-url', 'https://hub.example.test');
    expect(tv.app.settings.hubSignInUrl, 'https://hub.example.test');
    expect(key('hub-sign-in'), findsOneWidget);
  });

  appTest(
    'a signed-out Agent chat offers Sign in; New chat and the composer are off',
    (tester) async {
      await pumpApp(tester, mobile: true);
      unawaited(openAgent(tester.element(find.byType(HomeScreen))));
      await settle(tester);

      expect(key('agent-screen'), findsOneWidget);
      expect(key('agent-sign-in-prompt'), findsOneWidget);
      expect(find.textContaining('The agent is off'), findsNothing);
      expect(find.byType(AgentComposer), findsNothing);
      expect(
        tester.widget<IconBtn>(key('agent-new-chat-mobile')).onTap,
        isNull,
      );

      await tapAndSettle(tester, key('agent-sign-in'));
      expect(key('hub-sign-in'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  appTest('the phone New chat icon has a 44px touch area', (tester) async {
    await pumpApp(tester, mobile: true);
    unawaited(openAgent(tester.element(find.byType(HomeScreen))));
    await settle(tester);

    final size = tester.getSize(key('agent-new-chat-mobile'));
    expect(size.width, greaterThanOrEqualTo(44));
    expect(size.height, greaterThanOrEqualTo(44));
    expect(tester.takeException(), isNull);
  });

  appTest('the Android chat route respects system and keyboard insets', (
    tester,
  ) async {
    await pumpApp(tester, mobile: true);
    final dpr = tester.view.devicePixelRatio;
    tester.view.padding = FakeViewPadding(top: 24 * dpr, bottom: 16 * dpr);
    tester.view.viewPadding = FakeViewPadding(top: 24 * dpr, bottom: 16 * dpr);
    tester.view.viewInsets = FakeViewPadding(bottom: 240 * dpr);
    addTearDown(tester.view.reset);
    unawaited(openAgent(tester.element(find.byType(HomeScreen))));
    await settle(tester);

    final screen = tester.getRect(key('agent-screen'));
    expect(screen.top, greaterThanOrEqualTo(24));
    expect(screen.bottom, lessThanOrEqualTo(812 - 240));
    expect(tester.takeException(), isNull);
  });

  appTest(
    'Sync now waits for a connected sync, and signed-out has no Agent row',
    (tester) async {
      final tv = await pumpApp(tester, mobile: true);
      await openAiSettings(tester);
      expect(key('hub-sync-now'), findsNothing);
      expect(key('agent-retry-connection'), findsNothing);

      signInFixture(tv, account: 'breadcat');
      await settle(tester);
      // Signed in but the sync loop is not running in this fixture.
      expect(key('hub-sync-now'), findsOneWidget);
      expect(tester.widget<SecondaryButton>(key('hub-sync-now')).onTap, isNull);
      expect(find.text('Agent not connected yet.'), findsOneWidget);
      expect(key('agent-retry-connection'), findsNothing);
      expect(tester.takeException(), isNull);
    },
  );

  // The Retry button reruns the open against the loopback port, which
  // refuses at once (no Hub). The stale error is replaced by the new one.
  appTest('Retry connection reruns the agent open after a failure', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: true);
    final store = AgentStore.of(tv.app);
    tv.app.settings.update((s) {
      s.clientUrl = 'http://127.0.0.1:1';
      s.clientToken = 'test-device-token';
      s.clientDeviceLabel = 'Android';
    });
    store.openError = 'stale failure';
    await openAiSettings(tester);

    expect(find.text('Agent could not connect: stale failure'), findsOneWidget);
    await tapAndSettle(tester, key('agent-retry-connection'));
    await settleUntil(tester, () => store.openError != 'stale failure');
    expect(store.isOpen, isFalse);
    expect(store.openError, isNot('stale failure'));
    expect(find.textContaining('Agent could not connect:'), findsOneWidget);
    expect(find.text('Agent could not connect: stale failure'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  // A Hub that reports an account, and one that does not (older Hubs): the
  // older one shows the device name it registered, never a made-up account.
  for (final account in <String?>[null, 'breadcat']) {
    final label = account ?? 'older Hub';
    appTest('Hub sign in and sign out on Android ($label)', (tester) async {
      final tv = await pumpApp(tester, mobile: true);
      final store = AgentStore.of(tv.app);
      final auth = _FakeLogin();
      Navigator.of(tester.element(find.byType(HomeScreen))).push(
        PageRouteBuilder<void>(
          pageBuilder: (_, _, _) => _HubAgentPage(store, auth),
        ),
      );
      await settle(tester);

      await tapAndSettle(tester, key('hub-sign-in'));
      expect(auth.requestedUrl, 'https://notes.breadcat.cc');
      expect(find.text('Waiting for the browser…'), findsOneWidget);
      auth.pending!.complete(
        HubAuthResult(
          hubUrl: 'http://127.0.0.1:1',
          token: 'test-device-token',
          deviceId: tv.vault.deviceId,
          deviceName: 'Android',
          account: account,
        ),
      );
      await settleUntil(
        tester,
        () => key('hub-sign-out').evaluate().isNotEmpty,
      );
      expect(
        find.text('Signed in to 127.0.0.1 as ${account ?? 'Android'}'),
        findsOneWidget,
      );
      expect(key('hub-sign-in'), findsNothing);
      expect(tv.app.settings.clientToken, 'test-device-token');

      await tapAndSettle(tester, key('hub-sign-out'));
      expect(auth.revokedToken, 'test-device-token');
      expect(tv.app.settings.clientToken, isNull);
      expect(tv.app.settings.clientAccount, isNull);
      // Browser sign-in saves the address the Hub returned; sign-out keeps it.
      expect(tv.app.settings.hubSignInUrl, 'http://127.0.0.1:1');
      expect(key('hub-sign-in'), findsOneWidget);
      expect(tester.takeException(), isNull);
    });
  }

  appTest('a signed-in Hub shows its account and the models it exposes', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: true);
    final store = AgentStore.of(tv.app);
    signInFixture(tv, account: 'breadcat');
    store.models = _stubModels;
    await openAiSettings(tester);

    expect(find.text('Signed in to hub.test as breadcat'), findsOneWidget);
    expect(key('hub-sign-out'), findsOneWidget);
    expect(key('hub-sign-in'), findsNothing);
    // The fake state never connects: the status says so.
    expect(find.text('Not connected to the Hub right now.'), findsOneWidget);
    expect(key('agent-default-model'), findsOneWidget);
    expect(find.text('Fable 5.1'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  appTest('a signed-in Hub that reports no account shows the device name', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: true);
    signInFixture(tv);
    await openAiSettings(tester);

    expect(find.text('Signed in to hub.test as Android'), findsOneWidget);
    expect(key('agent-default-model'), findsNothing);
    expect(tester.takeException(), isNull);
  });
}
