// Conflicts from the settings side and the vault switch: a pending row in
// "Changed on two devices" opens its note; the auto-resolve switch is one
// vault-wide setting that reaches the other device; a text conflict resolves
// inline in dark mode.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/features/settings/controls.dart';
import 'package:notes_app/features/sync/conflicts.dart';
import 'package:notes_app/ui/format.dart';

import 'harness.dart';
import 'sync_helpers.dart';

const _topline = 'edited this section after you deleted it here';

void main() {
  setUpAll(initCore);

  appTest('a pending section row in settings opens its note with the topline', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final s = await makeSectionPage(tester, other, 'Lisbon trip');
      await makeSubtreeConflict(tester, other, s);

      await openConflictSettings(tester);
      final row = key('conflict-page-${s.page}');
      final c = tv.app.sync.conflictsOf(s.page).single;
      expect(row, findsOneWidget);
      expect(find.descendant(of: row, matching: find.text('Lisbon trip')), findsOneWidget);
      expect(find.descendant(of: row, matching: find.text('Cloud brought Packing back')), findsOneWidget);
      expect(find.descendant(of: row, matching: find.text(relativeTime(c.timeMs.toInt()))), findsOneWidget);

      await tapAndSettle(tester, row);
      expect(key('settings-nav-conflicts'), findsNothing, reason: 'the row opens the note');
      expect(tester.widgetList<PageScreen>(find.byType(PageScreen)).map((p) => p.pageId), contains(s.page));
      expect(find.textContaining(_topline), findsOneWidget);
      expect(find.text('Chargers, two adapters'), findsOneWidget);
      expect(tv.app.sync.conflictsOf(s.page), hasLength(1));
    } finally {
      await other.close(tester);
    }
  });

  appTest('a pending text row in settings shows its title, summary and time, and opens its note', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      await makeConflict(tester, other, 'Weekly review');
      final page = tv.pages['Weekly review']!;
      final c = tv.app.sync.conflictsOf(page).single;

      await openConflictSettings(tester);
      final row = key('conflict-page-$page');
      expect(find.descendant(of: row, matching: find.text('Weekly review')), findsOneWidget);
      expect(find.descendant(of: row, matching: find.text('Same line edited on both')), findsOneWidget);
      expect(find.descendant(of: row, matching: find.text(relativeTime(c.timeMs.toInt()))), findsOneWidget);

      await tapAndSettle(tester, row);
      expect(tester.widgetList<PageScreen>(find.byType(PageScreen)).map((p) => p.pageId), contains(page));
      final alt = conflictShownSide(c) == 0 ? 1 : 0;
      expect(find.text(c.sides[alt].block!.text), findsOneWidget);
    } finally {
      await other.close(tester);
    }
  });

  appTest('vault auto-resolve: on clears rule entries into the log; off waits for OK; the setting reaches the other device', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      expect(await core(tester, (v) => v.run((n) => n.conflictAutoResolve())), isFalse, reason: 'off by default');

      // On.
      final first = await makeSectionPage(tester, other, 'Lisbon trip');
      await openConflictSettings(tester);
      expect(tester.widget<Toggle>(key('conflict-auto-resolve')).value, isFalse, reason: 'the switch shows off');
      await tapAndSettle(tester, key('conflict-auto-resolve'));
      expect(tester.widget<Toggle>(key('conflict-auto-resolve')).value, isTrue);
      expect(await core(tester, (v) => v.run((n) => n.conflictAutoResolve())), isTrue);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(tester);

      await makeSubtreeConflict(tester, other, first);
      expect(tv.app.sync.conflicts, isEmpty, reason: 'the rule entry leaves the pending list');
      expect(tv.app.sync.conflictLogOf(first.page), hasLength(1));
      expect(tv.app.sync.conflictLogOf(first.page).single.applied.single.by, ResolutionBy.auto);
      expect((await blocksOf(tester, first.page)).map((b) => b.text), contains('Chargers, two adapters'));
      expect(await real(tester, () => other.client!.syncOnce()), isNotNull);
      expect(await real(tester, () => other.vault.run((v) => v.conflictAutoResolve())), isTrue,
          reason: 'the setting reached the other device');

      // Off.
      await openConflictSettings(tester);
      expect(tester.widget<Toggle>(key('conflict-auto-resolve')).value, isTrue, reason: 'reopened settings show the saved value');
      await tapAndSettle(tester, key('conflict-auto-resolve'));
      expect(tester.widget<Toggle>(key('conflict-auto-resolve')).value, isFalse);
      expect(await core(tester, (v) => v.run((n) => n.conflictAutoResolve())), isFalse);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(tester);

      final second = await makeSectionPage(tester, other, 'Budget notes');
      await makeSubtreeConflict(tester, other, second);
      expect(tv.app.sync.conflictsOf(second.page), hasLength(1), reason: 'waits in the pending list');
      expect(tv.app.sync.conflictLogOf(second.page), isEmpty);

      await openConflictPage(tester, second.page);
      await tapAndSettle(tester, key('keep-${second.root}'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expect(await real(tester, () => other.client!.syncOnce()), isNotNull);
      expect(await real(tester, () => other.vault.run((v) => v.conflictAutoResolve())), isFalse,
          reason: 'the switch is back off on the other device too');
      expect(await real(tester, () => other.vault.run((v) => v.listConflicts(page: null))), isEmpty);
    } finally {
      await other.close(tester);
    }
  });

  appTest('dark: a text conflict offers the other text inline and one tap takes it', (tester) async {
    final tv = await pumpApp(tester, mobile: false, dark: true);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final block = await makeConflict(tester, other, 'Weekly review');
      final page = tv.pages['Weekly review']!;
      await openConflictPage(tester, page);
      final c = tv.app.sync.conflictsOf(page).single;
      final alt = conflictShownSide(c) == 0 ? 1 : 0;
      final offered = c.sides[alt].block!.text;

      expect(MediaQuery.platformBrightnessOf(tester.element(find.text(offered))), Brightness.dark);
      expect(find.text(offered), findsOneWidget);
      await tapAndSettle(tester, find.text(offered), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expect((await blockById(tester, page, block)).text, offered);
    } finally {
      await other.close(tester);
    }
  });

  appTest('the switch follows a change made on the other device while settings stays open', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      await openConflictSettings(tester);
      expect(tester.widget<Toggle>(key('conflict-auto-resolve')).value, isFalse);

      // The other device turns the vault-wide switch on and syncs; settings
      // stays open here, with no reopen.
      await real(tester, () => other.vault.run((v) => v.setConflictAutoResolve(enabled: true), writes: true));
      await real(tester, () => other.client!.syncOnce());
      await settleUntil(tester, () => tester.widget<Toggle>(key('conflict-auto-resolve')).value);
      expect(tester.widget<Toggle>(key('conflict-auto-resolve')).value, isTrue, reason: 'the open switch updates itself');
      expect(await core(tester, (v) => v.run((n) => n.conflictAutoResolve())), isTrue);
      expect(tv.app.sync.conflicts, isEmpty);
    } finally {
      await other.close(tester);
    }
  });

  appTest('settings with nothing changed shows the short empty line', (tester) async {
    await pumpApp(tester, mobile: false);
    await openConflictSettings(tester);
    expect(key('no-conflicts'), findsOneWidget);
    expect(find.text('Nothing changed on two devices.'), findsOneWidget);
    expect(find.byType(Toggle), findsOneWidget, reason: 'the auto-resolve switch is still shown with nothing pending');
  });
}
