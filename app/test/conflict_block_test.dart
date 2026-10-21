// Block conflicts: both devices edit the same line. The note shows the line the
// core picks (conflictShownSide: the later edit, equal times to the greater
// device id), labelled Local or Cloud · from <device>. The other side's text is
// offered inline under its own label. One tap takes that text, and the toast's
// Undo restores the note's line as it was before the tap and brings the same
// conflict back as pending. A block conflict shows no topline.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/sync/conflict_copy.dart';
import 'package:notes_app/features/sync/conflicts.dart';

import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);

  appTest('block UseSide: the other text takes the line; Undo restores the before text and the same pending conflict', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final block = await makeConflict(tester, other, 'Weekly review');
      final page = tv.pages['Weekly review']!;
      final conflictId = tv.app.sync.conflictsOf(page).single.id;
      // The merged line is whatever the core shows first; read it, don't assume a side.
      final before = (await blockById(tester, page, block)).text;

      // Which side the note shows is the rule's answer, not a fixed side. The
      // conflict has two sides; the other side is the one offered inline.
      final c = tv.app.sync.conflictsOf(page).single;
      final shown = conflictShownSide(c)!;
      final alt = 1 - shown;
      final shownSide = c.sides[shown];
      final altSide = c.sides[alt];
      String labelOf(ConflictSide s) =>
          s.local ? conflictLocalLabel : conflictCloudLabel(tv.app.sync.deviceName(s.device));
      final shownLabel = labelOf(shownSide);
      final altLabel = labelOf(altSide);
      final altText = altSide.block!.text;

      await openConflictPage(tester, page);
      expect(editableWith(before), findsOneWidget, reason: 'the note shows the merged line first');
      expect(find.text(shownLabel), findsOneWidget, reason: 'the note line is labelled with the side it shows');
      expect(find.text(altLabel), findsOneWidget, reason: 'the other side is labelled under its own label');
      expect(find.text(altText), findsOneWidget, reason: 'the other text is offered inline');
      expect(
        tester.getTopLeft(find.text(shownLabel)).dx,
        closeTo(tester.getTopLeft(editableWith(before)).dx, 0.5),
        reason: 'the note label starts in the same gutter as the line',
      );

      // The toast is shown after the native take: wait for that work first.
      final tapped = altSide.local ? conflictUsedLocalLine : conflictUsedCloudLine;
      await tapAndSettle(tester, find.text(altText), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expect((await blockById(tester, page, block)).text, altText);
      expect(editableWith(before), findsNothing);
      expect(find.text(tapped), findsOneWidget, reason: 'the toast names the line that was used');
      expect(find.text('Undo'), findsOneWidget);

      // The toast sits inside the window.
      final view = tester.view.physicalSize / tester.view.devicePixelRatio;
      final toast = tester.getRect(key('conflict-toast'));
      expect(toast.left, greaterThanOrEqualTo(0));
      expect(toast.right, lessThanOrEqualTo(view.width));
      expect(toast.bottom, lessThanOrEqualTo(view.height));

      // A block has no topline: none of the page or section topline copy shows.
      expect(find.textContaining("so it's back"), findsNothing);
      expect(find.text('Use the local version'), findsNothing);

      await tapAndSettle(tester, find.text('Undo'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.length == 1);
      expect((await blockById(tester, page, block)).text, before, reason: 'Undo restores the before text');
      expect(editableWith(before), findsOneWidget);
      final back = tv.app.sync.conflicts.single;
      expect(back.id, conflictId, reason: 'the same conflict comes back');
      expect(back.status, ConflictStatus.pending);
      expect(back.applied.last.undoneAt, isNotNull, reason: 'the undone application stays in history');
      expect(key('conflict-toast'), findsNothing, reason: 'Undo hides the toast');
      expect(find.text(altText), findsOneWidget, reason: 'the other text is offered again');
      expect(key('keep-$block'), findsOneWidget, reason: 'OK is offered again');

      await real(tester, () => other.client!.syncOnce());
      final remote = await real(tester, () => other.vault.run((v) => v.listConflicts(page: null)));
      expect(remote.map((x) => x.id), [conflictId]);
    } finally {
      await other.close(tester);
    }
  });

  appTest('block: the note shows the later side\'s text', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final block = await makeConflict(tester, other, 'Weekly review');
      final page = tv.pages['Weekly review']!;
      final before = (await blockById(tester, page, block)).text;
      final c = tv.app.sync.conflictsOf(page).single;
      final shownSide = c.sides[conflictShownSide(c)!];
      expect(before, shownSide.block!.text, reason: 'the note line is the shown side\'s text');
    } finally {
      await other.close(tester);
    }
  });
}
