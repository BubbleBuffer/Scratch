// Golden screenshots of the chat in compacted mode, at desktop and phone
// width, light and dark: opened on the "Full messages from here" hairline, and
// scrolled up to the first closed chapter opened into its episodes with the
// first episode's action log. Baselines are rendered on the CI runner
// (Linux, the golden render step of the compacted-history workflow), not on
// Windows, so they are kept under goldens/compacted and compared by hand.
// Regenerate with: flutter test --update-goldens test/agent_history_golden_test.dart

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
        await pumpApp(tester, mobile: phone, empty: true, dark: dark);
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
