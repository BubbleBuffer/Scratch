// Where a conflict is entered from: the phone Home section ("Changed on two
// devices", with a count and a card per page) and the desktop rail group (the
// same count, rows hidden until its toggle opens them). A row opens its page.
// With nothing pending, neither entry is in the tree.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/editor/page_screen.dart';

import 'harness.dart';
import 'sync_helpers.dart';

Iterable<String> _pagesOpened(WidgetTester tester) =>
    tester.widgetList<PageScreen>(find.byType(PageScreen)).map((p) => p.pageId);

void main() {
  setUpAll(initCore);

  appTest('phone Home: the section counts the conflicted pages, shows a card per page, and a card opens its page', (tester) async {
    final tv = await pumpApp(tester, mobile: true);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final a = await makeSectionPage(tester, other, 'Lisbon trip');
      await makeSubtreeConflict(tester, other, a);
      final b = await makeSectionPage(tester, other, 'Budget notes');
      await makeSubtreeConflict(tester, other, b);
      expect(tv.app.sync.conflictPages, hasLength(2));

      await tester.ensureVisible(key('conflict-home'));
      final section = key('conflict-home');
      expect(find.descendant(of: section, matching: find.text('Changed on two devices')), findsOneWidget);
      expect(find.descendant(of: key('conflict-home-count'), matching: find.text('2')), findsOneWidget,
          reason: 'the count is the number of pages with pending conflicts');

      for (final (page, title) in [(a.page, 'Lisbon trip'), (b.page, 'Budget notes')]) {
        final row = key('conflict-home-page-$page');
        expect(find.descendant(of: row, matching: find.text(title)), findsOneWidget);
        expect(find.descendant(of: row, matching: find.textContaining('Cloud brought Packing back')), findsOneWidget);
      }

      final row = key('conflict-home-page-${a.page}');
      await tester.ensureVisible(row);
      await tapAndSettle(tester, row);
      expect(_pagesOpened(tester), contains(a.page), reason: 'the card opens its page');
    } finally {
      await other.close(tester);
    }
  });

  appTest('phone Home with nothing pending: no conflict section in the tree', (tester) async {
    await pumpApp(tester, mobile: true);
    expect(key('conflict-home'), findsNothing);
    expect(key('conflict-home-count'), findsNothing);
    expect(find.text('Changed on two devices'), findsNothing);
  });

  appTest('desktop rail: the group shows the count; rows stay hidden until the toggle opens them; a row opens its page', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final a = await makeSectionPage(tester, other, 'Lisbon trip');
      await makeSubtreeConflict(tester, other, a);
      final b = await makeSectionPage(tester, other, 'Budget notes');
      await makeSubtreeConflict(tester, other, b);
      expect(tv.app.sync.conflictPages, hasLength(2));

      expect(key('conflict-rail'), findsOneWidget);
      expect(find.descendant(of: key('conflict-rail-count'), matching: find.text('2')), findsOneWidget,
          reason: 'the count is the number of pages with pending conflicts');
      expect(key('conflict-rail-page-${a.page}'), findsNothing, reason: 'rows are hidden until the toggle is tapped');
      expect(key('conflict-rail-page-${b.page}'), findsNothing);

      await tapAndSettle(tester, key('conflict-rail-toggle'));
      final row = key('conflict-rail-page-${a.page}');
      expect(row, findsOneWidget);
      expect(find.descendant(of: row, matching: find.text('Lisbon trip')), findsOneWidget);
      expect(key('conflict-rail-page-${b.page}'), findsOneWidget);

      await tapAndSettle(tester, row);
      expect(_pagesOpened(tester), contains(a.page), reason: 'the row opens its page');
    } finally {
      await other.close(tester);
    }
  });

  appTest('desktop rail with nothing pending: no rail group in the tree', (tester) async {
    await pumpApp(tester, mobile: false);
    expect(key('conflict-rail'), findsNothing);
    expect(key('conflict-rail-count'), findsNothing);
    expect(key('conflict-rail-toggle'), findsNothing);
  });
}
