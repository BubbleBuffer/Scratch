// Page conflicts: this device trashed a page, or trashed and purged it, and the
// other device edited it. The editing version comes back on detection under a
// topline, which ends "so it's back in Pages." "OK" keeps the page and shows no
// toast. "Delete again" sends it back to Trash or purges it again, and that
// choice reaches the other device on its next sync.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';

import 'harness.dart';
import 'sync_helpers.dart';

const _changed = 'changed this page after you deleted it here';
const _purgedChanged = 'changed this page after it was permanently deleted here';
const _backInPages = "so it's back in Pages.";

/// A page action's toast: the line, the toast itself, and no Undo (the choice
/// is final for a page).
void expectPageToast(String copy) {
  expect(find.text(copy), findsOneWidget);
  expect(key('conflict-toast'), findsOneWidget);
  expect(find.text('Undo'), findsNothing);
  expect(key('conflict-toast-undo'), findsNothing);
}

void main() {
  setUpAll(initCore);

  appTest('trashed page: OK keeps the page with the cloud edit, and shows no toast', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final page = await makePageConflict(tester, other, 'Ideas for the zine', purge: false);
      await openConflictPage(tester, page);
      expect(find.textContaining(_changed), findsOneWidget);
      expect(find.textContaining(_backInPages), findsOneWidget, reason: 'the topline says the page is back in Pages');
      expect(find.text('Cloud text'), findsOneWidget);
      final c = tv.app.sync.conflictsOf(page).single;
      expect(c.scope.kind, ConflictScopeKind.page);
      expect(c.sides.map((x) => x.state.kind), containsAll([ConflictStateKind.trashed, ConflictStateKind.present]));

      await tapAndSettle(tester, key('keep-page'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expect(find.textContaining(_changed), findsNothing);
      expect(key('conflict-toast'), findsNothing, reason: 'OK on a page shows no toast');
      expect(find.text('Undo'), findsNothing);
      expect((await blocksOf(tester, page)).map((b) => b.text), ['Cloud text']);
      expect((await core(tester, (v) => v.listTrash())).map((t) => t.page.id), isNot(contains(page)));

      await real(tester, () => other.client!.syncOnce());
      expect(await real(tester, () => other.vault.run((v) => v.listConflicts(page: null))), isEmpty);
      expect((await real(tester, () => other.vault.listTrash())).map((t) => t.page.id), isNot(contains(page)));
    } finally {
      await other.close(tester);
    }
  });

  appTest('trashed page: Delete again sends the page back to Trash, on both devices', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final page = await makePageConflict(tester, other, 'Ideas for the zine', purge: false);
      await openConflictPage(tester, page);
      expect(find.textContaining(_changed), findsOneWidget);

      await tapAndSettle(tester, find.text('Delete again'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expectPageToast('Deleted again.');
      expect((await core(tester, (v) => v.listTrash())).map((t) => t.page.id), contains(page));

      await real(tester, () => other.client!.syncOnce());
      expect(await real(tester, () => other.vault.run((v) => v.listConflicts(page: null))), isEmpty);
      expect((await real(tester, () => other.vault.listTrash())).map((t) => t.page.id), contains(page));
    } finally {
      await other.close(tester);
    }
  });

  appTest('purged page: OK recreates the page with the cloud edit, and shows no toast', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final page = await makePageConflict(tester, other, 'Ideas for the zine', purge: true);
      await openConflictPage(tester, page);
      expect(find.textContaining(_purgedChanged), findsOneWidget);
      expect(find.textContaining(_backInPages), findsOneWidget, reason: 'the topline says the page is back in Pages');
      final c = tv.app.sync.conflictsOf(page).single;
      expect(c.sides.map((x) => x.state.kind), containsAll([ConflictStateKind.purged, ConflictStateKind.present]));

      await tapAndSettle(tester, key('keep-page'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expect(find.textContaining(_purgedChanged), findsNothing);
      expect(key('conflict-toast'), findsNothing, reason: 'OK on a page shows no toast');
      expect(find.text('Undo'), findsNothing);
      expect((await blocksOf(tester, page)).map((b) => b.text), ['Cloud text']);
      expect(await core(tester, (v) => v.run((n) => n.purgedPages())), isNot(contains(page)));

      await real(tester, () => other.client!.syncOnce());
      expect(await real(tester, () => other.vault.run((v) => v.listConflicts(page: null))), isEmpty);
    } finally {
      await other.close(tester);
    }
  });

  appTest('purged page: Delete again purges it again, on both devices', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final page = await makePageConflict(tester, other, 'Ideas for the zine', purge: true);
      await openConflictPage(tester, page);
      expect(find.textContaining(_purgedChanged), findsOneWidget);

      await tapAndSettle(tester, find.text('Delete again'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expectPageToast('Deleted again.');
      expect(await core(tester, (v) => v.run((n) => n.purgedPages())), contains(page));

      await real(tester, () => other.client!.syncOnce());
      expect(await real(tester, () => other.vault.run((v) => v.listConflicts(page: null))), isEmpty);
      expect(await real(tester, () => other.vault.run((v) => v.purgedPages())), contains(page));
    } finally {
      await other.close(tester);
    }
  });
}
