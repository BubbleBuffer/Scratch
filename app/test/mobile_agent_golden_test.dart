// Golden screenshots of the Android Hub agent at phone size (375x812 logical,
// light and dark), built from the real screens with fake state:
// - settings signed out: Settings > AI > Agent with the Hub address and Sign in;
// - chat signed out: the Agent screen's Sign in prompt, no composer;
// - settings signed in: the account, the connection line and the stub models.
// Regenerate the new files with:
//   flutter test --update-goldens test/mobile_agent_golden_test.dart

@Tags(['golden'])
library;

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/shell/home.dart';

import 'harness.dart';
import 'sync_helpers.dart';

Future<void> shot(String name) async {
  await expectLater(
    find.byType(Directionality).first,
    matchesGoldenFile('goldens/$name.png'),
  );
}

const _stubModels = [
  AgentModel(id: 'hub/fable', name: 'Fable 5.1'),
  AgentModel(id: 'hub/opus', name: 'Opus 5.5'),
];

Future<void> _openAiSettings(WidgetTester tester) async {
  await openMobileSettings(tester);
  await tapAndSettle(tester, key('settings-nav-ai'));
}

void main() {
  setUpAll(initCore);

  for (final dark in [false, true]) {
    final tag = 'phone-${dark ? 'dark' : 'light'}';

    appTest('mobile agent settings signed out $tag', (tester) async {
      await pumpApp(tester, mobile: true, dark: dark);
      await _openAiSettings(tester);
      await tester.ensureVisible(key('hub-sign-in'));
      await settle(tester);
      await shot('mobile-agent-settings-signed-out-$tag');
    });

    appTest('mobile agent chat signed out $tag', (tester) async {
      await pumpApp(tester, mobile: true, dark: dark);
      unawaited(openAgent(tester.element(find.byType(HomeScreen))));
      await settle(tester);
      expect(key('agent-sign-in-prompt'), findsOneWidget);
      await shot('mobile-agent-chat-signed-out-$tag');
    });

    appTest('mobile agent settings signed in $tag', (tester) async {
      final tv = await pumpApp(tester, mobile: true, dark: dark);
      final store = AgentStore.of(tv.app);
      tv.app.settings.update((s) {
        s.clientUrl = 'https://hub.test';
        s.clientToken = 'test-device-token';
        s.clientDeviceLabel = 'Android';
        s.clientAccount = 'alice';
      });
      store.models = _stubModels;
      await _openAiSettings(tester);
      await tester.ensureVisible(key('agent-hub-model'));
      await settle(tester);
      await shot('mobile-agent-settings-signed-in-$tag');
    });
  }
}
