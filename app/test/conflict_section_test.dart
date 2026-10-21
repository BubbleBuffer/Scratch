// Section conflicts: this device deleted a section and the other device
// edited a block inside it. The editing version is applied when the conflict
// is detected, so the note shows the section back under a topline. OK keeps
// it. "Use the local version" removes the section again, and the undo toast
// brings the conflict back as pending. Desktop and phone sizes.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';

import 'harness.dart';
import 'sync_helpers.dart';

const _topline = 'edited this section after you deleted it here';
const _removed = 'Packing section removed.';

void main() {
  setUpAll(initCore);

  appTest('section kept: OK keeps the edited section and clears the topline', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final s = await makeSectionPage(tester, other, 'Lisbon trip');
      await makeSubtreeConflict(tester, other, s);
      await openConflictPage(tester, s.page);

      expect(find.textContaining(_topline), findsOneWidget);
      expect(find.text('Packing'), findsOneWidget);
      expect(find.text('Chargers, two adapters'), findsOneWidget);
      expect(find.text('Rain jacket'), findsOneWidget);
      final c = tv.app.sync.conflictsOf(s.page).single;
      expect(c.scope.kind, ConflictScopeKind.subtree);
      expect(c.sides.map((x) => x.state.kind), containsAll([ConflictStateKind.deleted, ConflictStateKind.present]));

      // Waits for the native keep, so the absent toast is checked after the work.
      await tapAndSettle(tester, key('keep-${s.root}'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expect(find.textContaining(_topline), findsNothing);
      expect(key('conflict-toast'), findsNothing, reason: 'OK on a section shows no toast');
      expect(find.text('Undo'), findsNothing);
      expect((await blocksOf(tester, s.page)).map((b) => b.text), ['Packing', 'Chargers, two adapters', 'Rain jacket']);

      await real(tester, () => other.client!.syncOnce());
      expect(await real(tester, () => other.vault.run((v) => v.listConflicts(page: null))), isEmpty);
    } finally {
      await other.close(tester);
    }
  });

  appTest('section removed: "Use the local version" removes it; Undo brings the conflict back pending', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final s = await makeSectionPage(tester, other, 'Lisbon trip');
      await makeSubtreeConflict(tester, other, s);
      await openConflictPage(tester, s.page);

      // The toast is shown after the native removal: wait for that work, or
      // the six-second toast timer runs out during the settle.
      await tapAndSettle(tester, find.text('Use the local version'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expect(find.text('Packing'), findsNothing);
      expect(find.text('Chargers, two adapters'), findsNothing);
      expect(find.text(_removed), findsOneWidget);
      expect(key('conflict-toast'), findsOneWidget);
      expect((await blocksOf(tester, s.page)).map((b) => b.text), isNot(contains('Packing')));

      await tapAndSettle(tester, find.text('Undo'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.length == 1);
      expect(find.text(_removed), findsNothing);
      expect(find.textContaining(_topline), findsOneWidget, reason: 'the topline is back');
      expect(find.text('Chargers, two adapters'), findsOneWidget);
      final c = tv.app.sync.conflictsOf(s.page).single;
      expect(c.status, ConflictStatus.pending);
      expect(c.applied.last.undoneAt, isNotNull, reason: 'the undone application stays in history');

      await real(tester, () => other.client!.syncOnce());
      expect(await real(tester, () => other.vault.run((v) => v.listConflicts(page: null))), hasLength(1));
    } finally {
      await other.close(tester);
    }
  });

  appTest('phone: the section comes back under its topline; its removal can be undone', (tester) async {
    final tv = await pumpApp(tester, mobile: true);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final s = await makeSectionPage(tester, other, 'Lisbon trip');
      await makeSubtreeConflict(tester, other, s);
      await openConflictPage(tester, s.page, mobile: true);
      expect(find.textContaining(_topline), findsOneWidget);

      await tapAndSettle(tester, find.text('Use the local version'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expect(find.text(_removed), findsOneWidget);

      // The toast and its copy stay inside the phone width.
      final width = tester.view.physicalSize.width / tester.view.devicePixelRatio;
      final toast = tester.getRect(key('conflict-toast'));
      expect(toast.left, greaterThanOrEqualTo(0));
      expect(toast.right, lessThanOrEqualTo(width), reason: 'the toast fits the phone width');
      expect(tester.getRect(find.text(_removed)).right, lessThanOrEqualTo(width));

      await tapAndSettle(tester, find.text('Undo'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.length == 1);
      expect(find.textContaining(_topline), findsOneWidget);
      expect(find.text('Chargers, two adapters'), findsOneWidget);
    } finally {
      await other.close(tester);
    }
  });
}
