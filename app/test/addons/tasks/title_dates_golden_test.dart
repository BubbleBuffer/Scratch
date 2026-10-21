@Tags(['golden'])
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/registry/addon_registry.dart';

import '../../addon_harness.dart';
import 'title_dates_test.dart' show seedDatedNote, openNote;

// Source only: baseline images are pending review; do not render in this lane.
void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final name = '${mobile ? 'phone' : 'desktop'}-${dark ? 'dark' : 'light'}';
      appTest('note title dates $name', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark, empty: true);
        final page = await seedDatedNote(tester, [
          '2026-10-01',
          '2026-10-09',
          '2026-10-12',
        ]);
        await openNote(tester, page);
        await expectLater(
          find.byType(NoteTitleChips),
          matchesGoldenFile('goldens/note-title-dates-$name.png'),
        );
      });
    }
  }
}
