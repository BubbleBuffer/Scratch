// Golden screenshots of the chat in compacted mode, at desktop and phone
// width, light and dark: opened on the "Full messages from here" hairline, and
// scrolled up to the first closed chapter opened into its episodes with the
// first episode's action log. Baselines are Linux renders (not the Windows
// baselines of the other goldens), kept under goldens/compacted; the CI
// `flutter` job compares them on its pinned Flutter version (ci.yml). Regenerate
// on Linux with that Flutter: flutter test --update-goldens test/agent_history_golden_test.dart

@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agent_history_test.dart' show pumpHistoryChat, scrollUpTo, tapVisible;
import 'agent_test.dart' show openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

Future<void> _shot(String name) => expectLater(
  find.byKey(const ValueKey('history-shot')),
  matchesGoldenFile('goldens/compacted/$name.png'),
);

void main() {
  setUpAll(initCore);

  for (final phone in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${phone ? 'phone' : 'desktop'}-${dark ? 'dark' : 'light'}';

      Future<void> open(WidgetTester tester) async {
        // The phone is a 375 px box: the Android build talks to a hub, which a
        // local fixture session cannot use.
        await pumpApp(tester, mobile: false, empty: true, dark: dark);
        final store = await openAgentStore(tester);
        await real(tester, () => store.createSession(title: 'Long chat'));
        await pumpHistoryChat(
          tester,
          store,
          dark: dark,
          width: phone ? 375 : 900,
          height: phone ? 760 : 720,
        );
      }

      appTest('compacted on the boundary, $tag', (tester) async {
        await open(tester);
        await _shot('boundary-$tag');
      });

      appTest('compacted cards opened, $tag', (tester) async {
        await open(tester);
        await scrollUpTo(tester, key('history-chapter-ch1-toggle'));
        await tapVisible(tester, key('history-chapter-ch1-toggle'));
        await tapVisible(tester, key('history-episode-ep1-header'));
        await _shot('cards-$tag');
      });
    }
  }
}
