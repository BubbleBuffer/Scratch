// Settings > Conflicts, Recent: a resolved section conflict is listed there
// with its outcome, Undo returns it to the pending list as Undone, a row opens
// its page, and an empty log says so. Desktop size, as settings is opened from
// the sidebar.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/page_screen.dart';

import 'harness.dart';
import 'sync_helpers.dart';

Iterable<String> _pagesOpened(WidgetTester tester) =>
    tester.widgetList<PageScreen>(find.byType(PageScreen)).map((p) => p.pageId);

void main() {
  setUpAll(initCore);

  appTest('section kept with OK: its entry appears under Recent as Kept', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final s = await makeSectionPage(tester, other, 'Lisbon trip');
      await makeSubtreeConflict(tester, other, s);
      await openConflictPage(tester, s.page);
      final id = tv.app.sync.conflictsOf(s.page).single.id;

      await tapAndSettle(tester, key('keep-${s.root}'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);

      await openConflictSettings(tester);
      final row = key('conflict-log-${s.page}-$id');
      expect(row, findsOneWidget);
      expect(find.descendant(of: row, matching: find.text('Lisbon trip')), findsOneWidget);
      expect(find.descendant(of: row, matching: find.textContaining('Kept')), findsOneWidget);
    } finally {
      await other.close(tester);
    }
  });

  appTest('section removed with "Use the local version": Recent names the local choice', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final s = await makeSectionPage(tester, other, 'Lisbon trip');
      await makeSubtreeConflict(tester, other, s);
      await openConflictPage(tester, s.page);
      final id = tv.app.sync.conflictsOf(s.page).single.id;

      await tapAndSettle(tester, find.text('Use the local version'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);

      await openConflictSettings(tester);
      final row = key('conflict-log-${s.page}-$id');
      expect(row, findsOneWidget);
      expect(find.descendant(of: row, matching: find.textContaining('Used the local version')), findsOneWidget);
    } finally {
      await other.close(tester);
    }
  });

  appTest('cloud text chosen: Recent names the cloud choice', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      await makeConflict(tester, other, 'Weekly review');
      final page = tv.pages['Weekly review']!;
      final c = tv.app.sync.conflictsOf(page).single;
      final cloud = c.sides.indexWhere((s) => !s.local);
      // This test checks the log for the cloud choice whichever line the rule shows.
      await real(tester, () => tv.app.sync.resolve(c, Resolution(kind: ResolutionKind.useSide, side: cloud)));
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);

      await openConflictSettings(tester);
      final row = key('conflict-log-$page-${c.id}');
      await tester.ensureVisible(row);
      await settle(tester);
      expect(find.descendant(of: row, matching: find.textContaining('Used the cloud version')), findsOneWidget);
    } finally {
      await other.close(tester);
    }
  });

  appTest('Undo in Recent returns the conflict to pending; its row then shows Undone and no Undo button', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final s = await makeSectionPage(tester, other, 'Lisbon trip');
      await makeSubtreeConflict(tester, other, s);
      await openConflictPage(tester, s.page);
      final id = tv.app.sync.conflictsOf(s.page).single.id;
      await tapAndSettle(tester, find.text('Use the local version'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);

      await openConflictSettings(tester);
      await tapAndSettle(tester, key('conflict-log-undo-$id'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.length == 1);

      final c = tv.app.sync.conflicts.single;
      expect(c.id, id);
      expect(c.status, ConflictStatus.pending);
      expect(tv.app.sync.conflictsOf(s.page), hasLength(1));

      final row = key('conflict-log-${s.page}-$id');
      expect(row, findsOneWidget);
      expect(find.descendant(of: row, matching: find.textContaining('Undone')), findsOneWidget);
      expect(key('conflict-log-undo-$id'), findsNothing, reason: 'an undone entry has nothing left to undo');
    } finally {
      await other.close(tester);
    }
  });

  appTest('a row in Recent opens its page', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final s = await makeSectionPage(tester, other, 'Lisbon trip');
      await makeSubtreeConflict(tester, other, s);
      await openConflictPage(tester, s.page);
      final id = tv.app.sync.conflictsOf(s.page).single.id;
      await tapAndSettle(tester, key('keep-${s.root}'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);

      await openConflictSettings(tester);
      final row = key('conflict-log-${s.page}-$id');
      await tester.ensureVisible(row);
      await tapAndSettle(tester, row);
      expect(key('settings-nav-conflicts'), findsNothing, reason: 'the row opens the page');
      expect(_pagesOpened(tester), contains(s.page));
    } finally {
      await other.close(tester);
    }
  });

  appTest('with nothing resolved, Recent shows the empty line', (tester) async {
    await pumpApp(tester, mobile: false);
    await openConflictSettings(tester);
    expect(key('no-conflict-log'), findsOneWidget);
    expect(find.text('Nothing resolved in the last 30 days.'), findsOneWidget);
  });
}
