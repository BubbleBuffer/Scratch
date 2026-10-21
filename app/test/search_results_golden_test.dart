// Golden screenshots of the grouped search results as the app shows them: the
// desktop overlay (showSearchOverlay's SearchPanel) and the mobile SearchScreen,
// light and dark, over a seeded native vault with page rows and chat rows from
// the agent's index. Regenerate only these new goldens with:
// flutter test --concurrency=1 --tags golden --update-goldens test/search_results_golden_test.dart

@Tags(['golden'])
library;

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/shell/search.dart';
import 'package:notes_app/shell/search_groups.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';
import 'search_chat_fixture.dart';
import 'sync_helpers.dart';

const _query = 'drop';

/// The two page rows the query finds, by title.
const _pageTitles = ['Native drop plan', 'Drop zone notes'];

/// The chat's closed chapter and its section, as the fixture seeds them. The
/// labels are the chat's name, then the chapter's and section's names.
const _chapterName = 'Native drag-and-drop';
const _sectionName = 'Chrome live drops pass';

bool _isRowKey(Key? key) {
  if (key is! ValueKey<String>) return false;
  final value = key.value;
  return value.startsWith('hit-') &&
      !value.startsWith('hit-path-') &&
      !value.startsWith('hit-snippet-');
}

/// The result rows on screen, in tree order, by their keys.
List<String> _shownRowKeys() => [
  for (final e in find.byWidgetPredicate((w) => _isRowKey(w.key)).evaluate())
    (e.widget.key! as ValueKey<String>).value,
];

/// Waits until the index holds the two pages and the chat's two anchor rows
/// under their expected labels. Asserts their ranked order before a screen
/// opens, so a partially indexed fixture cannot become a golden reference.
Future<List<Hit>> _settledResults(
  WidgetTester tester,
  SeededChat chat,
  Map<String, String> pageIds,
) async {
  final vault = current.vault;
  final first = chat.messages.first.id;
  final chapterRow = 'agent/chapter:${chat.sessionId}:$first';
  final sectionRow = 'agent/section:${chat.sessionId}:$first:$first';
  final expected = {
    pageIds['Drop zone notes']!: 'Drop zone notes',
    pageIds['Native drop plan']!: 'Native drop plan',
    chapterRow: 'Trip planning › $_chapterName',
    sectionRow: 'Trip planning › $_chapterName › $_sectionName',
  };
  for (var i = 0; i < 200; i++) {
    await real(tester, () => chat.store.refreshSessions());
    await real(tester, () => vault.flush());
    final hits = (await real(
      tester,
      () => vault.run((v) => v.searchAdaptive(query: _query, limit: 100)),
    )).hits;
    final rows = flattenSections(groupSearchHits(hits));
    if (rows.length == expected.length &&
        rows.every((h) => expected[hitRowId(h)] == h.title)) {
      expect(rows.map(hitRowId), orderedEquals(expected.keys));
      return rows;
    }
    await real(
      tester,
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
  }
  fail('the index never settled on the expected search rows for "$_query"');
}

/// Switches the open app to the phone form factor, the way pumpApp lays it out.
Future<void> _switchToPhone(WidgetTester tester) async {
  debugDefaultTargetPlatformOverride = TargetPlatform.android;
  tester.view.devicePixelRatio = 2.0;
  tester.view.physicalSize = const Size(375 * 2, 812 * 2);
  tester.view.padding = const FakeViewPadding(top: 24 * 2, bottom: 16 * 2);
  tester.view.viewPadding = tester.view.padding;
  await settle(tester);
}

/// Types [text] into the field under [fieldKey].
Future<void> _type(WidgetTester tester, String fieldKey, String text) async {
  final field = find.descendant(
    of: find.byKey(ValueKey(fieldKey)),
    matching: find.byType(EditableText),
  );
  await tester.enterText(field, text);
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';

      appTest('search groups $tag', (tester) async {
        // The agent needs the desktop platform to open a native chat (phones
        // wait for a hub link), so the chat is seeded there and the phone view
        // is switched on afterwards; the search reads only the vault index.
        await pumpApp(tester, mobile: false, dark: dark, empty: true);
        final pageIds = <String, String>{};
        for (final title in _pageTitles) {
          pageIds[title] = await real(
            tester,
            () => current.vault.createPage(title: title),
          );
        }
        await real(tester, () => current.vault.flush());
        final chat = await seedChat(
          tester,
          sessionTitle: 'BubblePlay cursor',
          chapterTitle: 'Native drag-and-drop: Large-drop fix',
          chapterSummary: 'The stream moves in 4 MB chunks with backpressure.',
          sectionTitle: 'Native drag-and-drop: Chrome live drops pass',
          sectionSummary: 'Virtual-file drag from the web to the PC: built.',
          indexQuery: _query,
        );
        // The index is settled before any search runs, so the screen shows the
        // final rows and nothing is caught mid-sweep.
        final expected = await _settledResults(tester, chat, pageIds);

        if (mobile) await _switchToPhone(tester);
        final ctx = tester.element(find.byType(Overlay).first);
        if (mobile) {
          unawaited(
            pushScreen<void>(ctx, (_) => const SearchScreen(), name: '/search'),
          );
          await settle(tester);
          await _type(tester, 'search-field', _query);
        } else {
          unawaited(showSearchOverlay(ctx));
          await settle(tester);
          await _type(tester, 'search-overlay-field', _query);
        }
        await settleUntil(
          tester,
          () =>
              find
                  .byKey(const ValueKey('search-group-pages'))
                  .evaluate()
                  .isNotEmpty &&
              find
                  .byKey(const ValueKey('search-group-chats'))
                  .evaluate()
                  .isNotEmpty,
        );
        await settle(tester);
        // The rows on screen, in order, are the core's order for the settled
        // results: pages, then chats.
        expect(_shownRowKeys(), [
          for (final h in expected) 'hit-${hitRowId(h)}',
        ]);
        await expectLater(
          find.byType(Directionality).first,
          matchesGoldenFile('../screenshots/$tag-search-groups.png'),
        );
        // The overlay's open flag is static: close it so the next test can open
        // its own.
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await settle(tester);
      });
    }
  }
}
