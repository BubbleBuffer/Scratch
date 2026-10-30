// Helpers for the sync tests: a second vault in a temporary folder (the
// "other device"), a client or hub on it, and a conflict made the way two
// devices make one. Hubs listen on 127.0.0.1 only.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/src/rust/api/sync.dart' show redeemPairingCode;

import 'harness.dart';

/// The other device: a vault in its own temporary folder.
class OtherDevice {
  OtherDevice(this.dir, this.vault);
  final Directory dir;
  final Vault vault;
  NotesSync? client;
  NotesHub? hub;

  Future<void> close(WidgetTester tester) async {
    await tester.runAsync(() async {
      final c = client, h = hub;
      if (c != null) {
        await c.stop();
        c.dispose();
      }
      if (h != null) {
        await h.stop();
        h.dispose();
      }
      await vault.flush();
      vault.dispose();
      try {
        await dir.delete(recursive: true);
      } catch (_) {}
    });
  }
}

Future<OtherDevice> openOtherDevice(WidgetTester tester) async {
  final d = await tester.runAsync(() async {
    final dir = Directory.systemTemp.createTempSync('notes_other_');
    final v = await Vault.open('${dir.path}${Platform.pathSeparator}vault');
    return OtherDevice(dir, v);
  });
  return d!;
}

/// Runs a bridge call from the test on the real event loop.
Future<T> real<T>(WidgetTester tester, Future<T> Function() f) async => (await tester.runAsync(f)) as T;

/// The app hosts (127.0.0.1, a free port); [other] pairs as "Phone" and does
/// a first full sync. Returns the hub URL.
Future<String> pairOther(WidgetTester tester, OtherDevice other) async {
  final sync = current.app.sync;
  final bound = await real(tester, () => sync.startHub('127.0.0.1:0'));
  final code = await real(tester, () => sync.pair('Phone'));
  final url = 'http://$bound';
  final token = await real(
    tester,
    () => redeemPairingCode(vault: other.vault.raw, hubUrl: url, code: code, trusted: const []),
  );
  other.client = NotesSync(vault: other.vault.raw, url: url, token: token);
  await real(tester, () => other.client!.syncOnce());
  await settle(tester);
  return url;
}

/// Both devices change the first block of [title] differently; the other
/// device syncs, so both vaults record a conflict. Returns the block id.
/// With [base], the block is first set to it and synced, so edits at
/// different places merge in a fixed order (same-place inserts merge in
/// peer-id order, which differs per run).
Future<String> makeConflict(WidgetTester tester, OtherDevice other, String title,
    {String here = 'Review on Monday', String there = 'Review on Friday', String? base}) async {
  final page = current.pages[title]!;
  final blocks = await blocksOf(tester, page);
  final b = blocks.first;
  BOp setText(String t) => BOp(op: OpKind.setText, id: b.id, kind: b.kind, text: t, marks: const [], checked: false);
  if (base != null) {
    await core(tester, (v) => v.apply(page, [setText(base)]));
    await real(tester, () => current.vault.flush());
    await real(tester, () => other.client!.syncOnce());
  }
  await core(tester, (v) => v.apply(page, [setText(here)]));
  // Loro stamps each change in whole seconds. When both edits share a second,
  // the shown text falls to the device-id tie-break, and the device id is
  // random per vault, so the text would differ between runs. Flushing the
  // first edit and waiting past a second boundary makes the later edit win.
  await real(tester, () => current.vault.flush());
  await real(tester, () => Future<void>.delayed(const Duration(milliseconds: 1100)));
  await real(tester, () => other.vault.apply(page, [setText(there)]));
  await real(tester, () => current.vault.flush());
  await real(tester, () => other.vault.flush());
  final r = await real(tester, () => other.client!.syncOnce());
  expect(r.conflicts, greaterThan(0), reason: 'the sync reports the conflict');
  await settleUntil(tester, () => current.app.sync.conflicts.isNotEmpty);
  expect(current.app.sync.conflicts, isNotEmpty, reason: 'the app sees the conflict');
  return b.id;
}

// ---------------------------------------------------------------------------
// Scoped conflicts on the real core, made the way two devices make them (the
// same sequences as crates/core/tests/conflict_detection.rs and
// conflict_resolutions.rs). The app's vault is the device that deletes or
// trashes; the other device edits and syncs. The app then sees the conflict
// through the hub it hosts.

/// A page with a section: a heading [root] and two blocks under it.
class SectionFixture {
  SectionFixture({required this.page, required this.root, required this.first, required this.second});
  final String page;
  final String root;
  final String first;
  final String second;
}

BOp insertBlock(String id, {String? parent, String? after, required String kind, required String text}) =>
    BOp(op: OpKind.insert, id: id, parent: parent, after: after, kind: kind, text: text, marks: const [], checked: false);

BOp setBlockText(String id, String kind, String text) =>
    BOp(op: OpKind.setText, id: id, kind: kind, text: text, marks: const [], checked: false);

/// Creates a page titled [title] with a section and syncs it to [other], so
/// both devices start from the same blocks.
Future<SectionFixture> makeSectionPage(WidgetTester tester, OtherDevice other, String title) async {
  final root = newId(), first = newId(), second = newId();
  final page = await core(tester, (v) => v.createPage(title: title));
  await core(tester, (v) => v.apply(page, [
        insertBlock(root, kind: Kinds.heading2, text: 'Packing'),
        insertBlock(first, parent: root, kind: Kinds.bulleted, text: 'Chargers and adapter'),
        insertBlock(second, parent: root, after: first, kind: Kinds.bulleted, text: 'Rain jacket'),
      ]));
  await real(tester, () => current.vault.flush());
  await real(tester, () => other.client!.syncOnce());
  await settle(tester);
  return SectionFixture(page: page, root: root, first: first, second: second);
}

/// A subtree conflict: this device deletes the section, the other device
/// edits a block inside it, and the other device syncs. Edit beats delete, so
/// the editing version is applied here and the conflict is recorded.
Future<void> makeSubtreeConflict(WidgetTester tester, OtherDevice other, SectionFixture s) async {
  await core(tester, (v) => v.apply(s.page, [
        BOp(op: OpKind.delete, id: s.root, kind: Kinds.heading2, text: '', marks: const [], checked: false),
      ]));
  await real(tester, () => current.vault.flush());
  await real(tester, () => other.vault.apply(s.page, [setBlockText(s.first, Kinds.bulleted, 'Chargers, two adapters')]));
  await real(tester, () => other.vault.flush());
  await real(tester, () => other.client!.syncOnce());
  await settleUntil(tester, () => current.app.sync.conflicts.isNotEmpty || current.app.sync.conflictLog.isNotEmpty);
}

/// A page conflict: this device trashes the page (or purges it, after
/// trashing), the other device edits its only block and syncs. The editing
/// version is restored here and the conflict is recorded at page scope.
/// Returns the page id.
Future<String> makePageConflict(WidgetTester tester, OtherDevice other, String title, {required bool purge}) async {
  final block = newId();
  final page = await core(tester, (v) => v.createPage(title: title));
  await core(tester, (v) => v.apply(page, [insertBlock(block, kind: Kinds.paragraph, text: 'Original text')]));
  await real(tester, () => current.vault.flush());
  await real(tester, () => other.client!.syncOnce());
  await settle(tester);
  await core(tester, (v) => v.deletePage(page));
  if (purge) {
    await core(tester, (v) => v.run((n) => n.purgePage(id: page), writes: true));
  }
  await real(tester, () => current.vault.flush());
  await real(tester, () => other.vault.apply(page, [setBlockText(block, Kinds.paragraph, 'Cloud text')]));
  await real(tester, () => other.vault.flush());
  await real(tester, () => other.client!.syncOnce());
  await settleUntil(tester, () => current.app.sync.conflicts.isNotEmpty || current.app.sync.conflictLog.isNotEmpty);
  return page;
}

/// Opens [page] from the page tree (desktop) or the phone's page list.
Future<void> openConflictPage(WidgetTester tester, String page, {bool mobile = false}) async {
  if (mobile) await openMobileTree(tester);
  final row = key(mobile ? 'pages-$page' : 'tree-$page');
  await tester.ensureVisible(row);
  await tapAndSettle(tester, row);
}

/// Opens Settings at its "Changed on two devices" section.
Future<void> openConflictSettings(WidgetTester tester, {bool mobile = false}) async {
  if (mobile) {
    await openMobileSettings(tester);
  } else {
    await tapAndSettle(tester, key('sidebar-settings'));
  }
  await tapAndSettle(tester, key('settings-nav-conflicts'));
}

Finder key(String k) => find.byKey(ValueKey(k));

/// The text of a Text widget found by key.
String textOf(WidgetTester tester, String k) => tester.widget<Text>(key(k)).data!;

/// Enters [text] into the PlainField with key [k].
Future<void> enterField(WidgetTester tester, String k, String text) async {
  await tester.enterText(find.descendant(of: key(k), matching: find.byType(EditableText)), text);
  await settle(tester, min: Duration.zero);
}
