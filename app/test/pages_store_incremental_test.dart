// PagesStore kept up to date by the change feed, checked against a fresh
// PagesStore on the same real vault after every settled window.
//
// Each seed builds a vault of a few hundred pages: documents (some nested)
// and four databases with fifty rows each. The primary sequence is a fixed
// script (content edits, a burst, renames, creates, deletes, moves, type
// changes in both directions) followed by seeded random windows. After every
// window the store under test must match a fresh PagesStore in full: its
// count, every row (all indices) and the full documents list, compared field
// by field.
//
// Window rules, by the feed's own classification of the batches:
// - An in-place window (no tree change) is one batch, notifies once,
//   calls no pageCount, and its listPages requests start at offset 0 with a
//   limit no larger than the number of changed pages. Those rules are checked
//   before the pageCount check, so a full reload fails on the first request.
// - A tree-changing window may reload fully; only the comparison applies.
//
// The feed's periodic and wake polls are pushed out to hours, and every poll
// is explicit, so a burst of writes is one batch. Debounce.instant stays
// false: with it true, the feed's wake delay is zero and a burst would split.
// The store's reload then waits its 400 ms debounce, which settle waits for.
//
// Prints one CHANGE_KINDS line per window (op, batches, change kinds, list
// requests, pageCount) so the metadata each op produces can be reviewed.
//
// The hidden calendars system page (role "calendars") must never appear in the
// core's lists, its count, the store's rows or its documents. Category writes
// and database calendar visibility change that page, so the scripted and
// random windows include them, alone and mixed with visible content writes.
// The feed cannot patch those changes, so the store reloads: that is checked
// for correctness only, never for the in-place cost rules. The hidden page is
// found through the core's role lookup, not through the store or the oracle.

import 'dart:async';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/change_feed.dart';
import 'package:notes_app/core/debounce.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/state/pages_store.dart';

import 'harness.dart' show initCore;

const seeds = [11, 22];
const randomWindows = 12;
const docTop = 80;
const docNested = 40;
const databases = 4;
const rowsPerDatabase = 50;
const opKinds = [
  'content',
  'content',
  'content',
  'rename',
  'rename',
  'create',
  'delete',
  'move',
  'toDatabase',
  'toDocument',
];
const calendarColors = ['apricot', 'sky', 'duvet'];

class ListRequest {
  const ListRequest(this.offset, this.limit, this.returned);
  final int offset;
  final int limit;
  final int returned;

  @override
  String toString() => 'offset=$offset,limit=$limit,returned=$returned';
}

/// Counts pageCount calls and records every listPages request.
class CountingVault extends Vault {
  CountingVault(super.raw);

  int pageCountCalls = 0;
  String? metaGatePage;
  Completer<void>? metaGate;
  Completer<void>? metaStarted;
  Completer<void>? countGate;
  Completer<void>? countStarted;
  Completer<void>? listGate;
  Completer<void>? listStarted;
  final requests = <ListRequest>[];

  /// Runs after each pageCount (call 'pageCount', offset and limit 0) and
  /// listPages response, before it returns to the store.
  Future<void> Function(String call, int offset, int limit)? onResponse;

  void resetCounts() {
    pageCountCalls = 0;
    requests.clear();
  }

  @override
  Future<PageData> loadPage(String id) async {
    final page = await super.loadPage(id);
    if (metaGatePage == id) {
      metaGatePage = null;
      metaStarted!.complete();
      await metaGate!.future;
    }
    return page;
  }

  @override
  Future<int> pageCount() async {
    pageCountCalls++;
    final count = await super.pageCount();
    if (countGate != null) {
      if (!countStarted!.isCompleted) countStarted!.complete();
      await countGate!.future;
    }
    await onResponse?.call('pageCount', 0, 0);
    return count;
  }

  @override
  Future<List<PageRow>> listPages({
    String? parent,
    required int offset,
    required int limit,
  }) async {
    final rows = await super.listPages(
      parent: parent,
      offset: offset,
      limit: limit,
    );
    requests.add(ListRequest(offset, limit, rows.length));
    if (limit == 1 && listGate != null) {
      if (!listStarted!.isCompleted) listStarted!.complete();
      await listGate!.future;
    }
    await onResponse?.call('listPages', offset, limit);
    return rows;
  }
}

class Coverage {
  int ordinary = 0; // windows with no tree change
  int tree = 0; // windows with a tree change
  int hidden = 0; // windows that wrote the hidden calendars page
  int rows = 0;
  int docs = 0;

  @override
  String toString() =>
      'ordinary=$ordinary tree=$tree hidden=$hidden rows=$rows docs=$docs';
}

BOp edit(String text) => BOp(
  op: OpKind.insert,
  id: newId(),
  kind: Kinds.paragraph,
  text: text,
  marks: const [],
  checked: false,
);

/// Builds the seeded vault. [ids] maps each seeded title to its page id; the
/// script refers to pages by these names, which stay fixed across renames.
Future<void> seedVault(
  CountingVault vault,
  math.Random rng,
  Map<String, String> ids,
) async {
  final top = <String>[];
  for (var i = 0; i < docTop; i++) {
    final id = await vault.createPage(title: 'Doc $i');
    top.add(id);
    ids['Doc $i'] = id;
  }
  for (var i = 0; i < docNested; i++) {
    ids['Nested $i'] = await vault.createPage(
      parent: top[rng.nextInt(top.length)],
      title: 'Nested $i',
    );
  }
  for (var d = 0; d < databases; d++) {
    final db = await vault.createPage(title: 'Database $d');
    ids['Database $d'] = db;
    await vault.run((v) => v.makeCollection(page: db), writes: true);
    for (var r = 0; r < rowsPerDatabase; r++) {
      ids['Row $d.$r'] = await vault.createPage(parent: db, title: 'Row $d.$r');
    }
  }
  await vault.flush();
}

/// Waits until the bridge and the debounced reloads are quiet. Polls the feed
/// explicitly at settle boundaries; store read barriers can also deliver batches.
Future<void> settle(ChangeFeed feed) async {
  var quiet = 0;
  final sw = Stopwatch()..start();
  while (sw.elapsed < const Duration(seconds: 30)) {
    await feed.poll();
    await Future<void>.delayed(const Duration(milliseconds: 2));
    final busy = Vault.inFlight.value > 0 || Debounce.pending > 0;
    quiet = busy ? 0 : quiet + 1;
    if (quiet >= 3) return;
  }
  fail(
    'did not settle: ${Vault.inFlight.value} bridge calls, ${Debounce.pending} debounces pending',
  );
}

/// Row [i] once the store has its chunk (asking starts the load).
Future<PageRow> loadedRow(PagesStore s, int i) async {
  final sw = Stopwatch()..start();
  PageRow? r;
  while ((r = s.rowAt(i)) == null) {
    if (sw.elapsed > const Duration(seconds: 20)) fail('row $i never loaded');
    await Future<void>.delayed(const Duration(milliseconds: 2));
  }
  return r!;
}

Future<List<PageRow>> allRows(PagesStore s) async => [
  for (var i = 0; i < s.count; i++) await loadedRow(s, i),
];

/// Every document of [s], scanning on until the store reports docComplete.
Future<List<PageRow>> allDocs(PagesStore s, ChangeFeed feed) async {
  for (var guard = 0; !s.docComplete; guard++) {
    if (guard > 1000) fail('document scan never finished');
    s.docAt(s.docCount);
    await settle(feed);
  }
  return [for (var i = 0; i < s.docCount; i++) s.docAt(i)!];
}

/// Every field of a row, properties included (compared by value, not identity).
void expectSameRow(PageRow a, PageRow b, String where) {
  expect(a.id, b.id, reason: '$where: id');
  expect(a.title, b.title, reason: '$where: title');
  expect(a.parent, b.parent, reason: '$where: parent');
  expect(a.updatedMs, b.updatedMs, reason: '$where: updatedMs');
  expect(a.createdMs, b.createdMs, reason: '$where: createdMs');
  expect(a.childCount, b.childCount, reason: '$where: childCount');
  expect(a.icon, b.icon, reason: '$where: icon');
  expect(a.pageType, b.pageType, reason: '$where: pageType');
  expect(a.favorite, b.favorite, reason: '$where: favorite');
  expect(
    a.properties.length,
    b.properties.length,
    reason: '$where: property count',
  );
  for (var k = 0; k < a.properties.length; k++) {
    final pa = a.properties[k];
    final pb = b.properties[k];
    final w = '$where: property $k';
    expect(pa.key, pb.key, reason: '$w key');
    expect(pa.value.kind, pb.value.kind, reason: '$w kind');
    expect(pa.value.text, pb.value.text, reason: '$w text');
    expect(pa.value.number, pb.value.number, reason: '$w number');
    expect(pa.value.startMs, pb.value.startMs, reason: '$w startMs');
    expect(pa.value.endMs, pb.value.endMs, reason: '$w endMs');
    expect(pa.value.hasTime, pb.value.hasTime, reason: '$w hasTime');
    expect(pa.value.checked, pb.value.checked, reason: '$w checked');
    expect(pa.value.ids, equals(pb.value.ids), reason: '$w ids');
  }
}

void expectSameRows(List<PageRow> a, List<PageRow> b, String where) {
  expect(a.length, b.length, reason: '$where: length');
  for (var i = 0; i < a.length; i++) {
    expectSameRow(a[i], b[i], '$where [$i]');
  }
}

/// Checks [store] against a fresh store on the same vault: all rows and the
/// full documents list.
Future<void> compareWithFresh({
  required CountingVault vault,
  required ChangeFeed feed,
  required PagesStore store,
  required String label,
  required Coverage cov,
  String? hidden,
}) async {
  final fresh = PagesStore(vault, feed);
  try {
    await fresh.reload();
    final freshRows = await allRows(fresh);
    final freshDocs = await allDocs(fresh, feed);
    expect(store.count, freshRows.length, reason: '$label: count');
    final rows = await allRows(store);
    final docs = await allDocs(store, feed);
    expectSameRows(rows, freshRows, '$label rows');
    expectSameRows(docs, freshDocs, '$label documents');
    if (hidden != null) {
      expect(freshRows.any((r) => r.id == hidden), isFalse,
          reason: '$label: fresh rows show the hidden page');
      expect(rows.any((r) => r.id == hidden), isFalse,
          reason: '$label: store rows show the hidden page');
      expect(docs.any((r) => r.id == hidden), isFalse,
          reason: '$label: store documents show the hidden page');
    }
    cov.rows += rows.length;
    cov.docs += docs.length;
  } finally {
    fresh.dispose();
  }
}

/// The hidden calendars system page, found through the core's role lookup.
Future<String> hiddenCalendarsPage(CountingVault vault) async {
  final id = await vault.run((v) => v.findByRole(role: 'calendars'));
  if (id == null) fail('the calendars system page does not exist');
  return id;
}

/// The core's own listing and count (not the oracle) hide [hidden], the store
/// count agrees with them, and the role lookup still finds the same page.
Future<void> expectCoreExcludes({
  required CountingVault vault,
  required PagesStore store,
  required String hidden,
  required String label,
}) async {
  final listed = await vault.listPages(offset: 0, limit: 100000);
  expect(listed.any((r) => r.id == hidden), isFalse,
      reason: '$label: core list shows the hidden page');
  expect(await vault.pageCount(), listed.length,
      reason: '$label: core count includes the hidden page');
  expect(store.count, listed.length, reason: '$label: store count');
  expect(await hiddenCalendarsPage(vault), hidden,
      reason: '$label: the calendars page moved');
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initCore);

  test(
    'content edits preserve sparse chunks and the partial document scan',
    () async {
      Debounce.instant = false;
      final dir = await Directory.systemTemp.createTemp('notes-pages-sparse-');
      final vault = CountingVault((await Vault.open(dir.path)).raw);
      final feed = ChangeFeed(
        vault,
        interval: const Duration(hours: 1),
        wakeDelay: const Duration(hours: 1),
      );
      final store = PagesStore(vault, feed);
      try {
        for (var i = 0; i < 650; i++) {
          await vault.createPage(title: 'Sparse $i');
        }
        await vault.flush();
        await feed.start();
        await store.reload();
        expect(store.docComplete, isFalse);
        final edited = await loadedRow(store, 450);
        // Chunk 3 has never been loaded, so shifting chunk 4 needs one boundary row.
        vault.resetCounts();
        var notifications = 0;
        void changed() => notifications++;
        store.addListener(changed);
        await vault.apply(edited.id, [edit('outside the scanned prefix')]);
        await settle(feed);
        store.removeListener(changed);
        expect(vault.pageCountCalls, 0);
        expect(vault.requests.length, 2);
        expect(vault.requests.first.offset, 0);
        expect(vault.requests.first.limit, 1);
        expect(vault.requests.last.offset, 400);
        expect(vault.requests.last.limit, 1);
        expect(notifications, 1);
        expect(store.docComplete, isFalse);
        expect(store.rowAt(0)!.id, edited.id);
        expect(store.rowAt(451), isNotNull, reason: 'loaded rows stay loaded');
        expect(store.docAt(0)!.id, edited.id);
        await compareWithFresh(
          vault: vault,
          feed: feed,
          store: store,
          label: 'sparse edit',
          cov: Coverage(),
        );
      } finally {
        store.dispose();
        feed.stop();
        vault.dispose();
        await dir.delete(recursive: true);
      }
    },
    timeout: const Timeout(Duration(minutes: 5)),
  );

  test(
    'a newer batch supersedes a pending content refresh without losing edits',
    () async {
      Debounce.instant = false;
      final dir = await Directory.systemTemp.createTemp('notes-pages-overlap-');
      final vault = CountingVault((await Vault.open(dir.path)).raw);
      final feed = ChangeFeed(
        vault,
        interval: const Duration(hours: 1),
        wakeDelay: const Duration(hours: 1),
      );
      final store = PagesStore(vault, feed);
      try {
        final first = await vault.createPage(title: 'First');
        final second = await vault.createPage(title: 'Second');
        await vault.createPage(title: 'Third');
        await vault.flush();
        await feed.start();
        await store.reload();
        vault.resetCounts();
        vault.listGate = Completer<void>();
        vault.listStarted = Completer<void>();
        var notifications = 0;
        void changed() => notifications++;
        store.addListener(changed);
        await vault.apply(first, [edit('first batch')]);
        await feed.poll();
        await vault.listStarted!.future.timeout(const Duration(seconds: 10));
        await vault.apply(second, [edit('second batch')]);
        await feed.poll();
        vault.listGate!.complete();
        await settle(feed);
        store.removeListener(changed);
        expect(vault.pageCountCalls, 0);
        expect(notifications, 1, reason: 'only the current refresh publishes');
        expect(store.rowAt(0)!.id, second);
        expect(store.rowAt(1)!.id, first);
        await compareWithFresh(
          vault: vault,
          feed: feed,
          store: store,
          label: 'overlapping batches',
          cov: Coverage(),
        );
      } finally {
        if (vault.listGate != null && !vault.listGate!.isCompleted) {
          vault.listGate!.complete();
        }
        store.dispose();
        feed.stop();
        vault.dispose();
        await dir.delete(recursive: true);
      }
    },
    timeout: const Timeout(Duration(minutes: 2)),
  );

  test(
    'a content batch cannot cancel the requirement for a structural reload',
    () async {
      Debounce.instant = false;
      final dir = await Directory.systemTemp.createTemp(
        'notes-pages-structural-overlap-',
      );
      final vault = CountingVault((await Vault.open(dir.path)).raw);
      final feed = ChangeFeed(
        vault,
        interval: const Duration(hours: 1),
        wakeDelay: const Duration(hours: 1),
      );
      final store = PagesStore(vault, feed);
      try {
        final first = await vault.createPage(title: 'First');
        await vault.flush();
        await feed.start();
        await store.reload();
        vault.resetCounts();
        vault.countGate = Completer<void>();
        vault.countStarted = Completer<void>();
        await vault.createPage(title: 'New page');
        await feed.poll();
        await vault.countStarted!.future.timeout(const Duration(seconds: 10));
        await vault.apply(first, [edit('content during structural reload')]);
        await feed.poll();
        vault.countGate!.complete();
        await settle(feed);
        expect(
          store.count,
          2,
          reason: 'the cancelled reload must still be required',
        );
        expect(vault.pageCountCalls, 2);
        await compareWithFresh(
          vault: vault,
          feed: feed,
          store: store,
          label: 'structural overlap',
          cov: Coverage(),
        );
      } finally {
        if (vault.countGate != null && !vault.countGate!.isCompleted) {
          vault.countGate!.complete();
        }
        store.dispose();
        feed.stop();
        vault.dispose();
        await dir.delete(recursive: true);
      }
    },
    timeout: const Timeout(Duration(minutes: 2)),
  );

  for (final cached in [false, true]) {
    test(
      'a late metadata read cannot overwrite an edit (cached=$cached)',
      () async {
        Debounce.instant = false;
        final dir = await Directory.systemTemp.createTemp(
          'notes-pages-meta-race-',
        );
        final vault = CountingVault((await Vault.open(dir.path)).raw);
        final feed = ChangeFeed(
          vault,
          interval: const Duration(hours: 1),
          wakeDelay: const Duration(hours: 1),
        );
        final store = PagesStore(vault, feed);
        try {
          final id = await vault.createPage();
          final block = edit('Old first line');
          await vault.apply(id, [block]);
          await vault.flush();
          final old = await vault.loadPage(id);
          await feed.start();
          await store.reload();
          vault.metaGatePage = id;
          vault.metaGate = Completer<void>();
          vault.metaStarted = Completer<void>();
          expect(store.meta(id), isNull);
          await vault.metaStarted!.future.timeout(const Duration(seconds: 10));
          if (cached) store.adopt(old);
          await vault.apply(id, [
            BOp(
              op: OpKind.setText,
              id: block.id,
              text: 'New first line',
              kind: Kinds.paragraph,
              marks: const [],
              checked: false,
            ),
          ]);
          await settle(feed);
          vault.metaGate!.complete();
          await settle(feed);
          final wait = Stopwatch()..start();
          while (store.meta(id) == null) {
            if (wait.elapsed > const Duration(seconds: 10)) {
              fail('metadata never refreshed');
            }
            await Future<void>.delayed(const Duration(milliseconds: 2));
          }
          expect(store.meta(id)!.firstLine, 'New first line');
        } finally {
          if (vault.metaGate != null && !vault.metaGate!.isCompleted) {
            vault.metaGate!.complete();
          }
          store.dispose();
          feed.stop();
          vault.dispose();
          await dir.delete(recursive: true);
        }
      },
      timeout: const Timeout(Duration(minutes: 2)),
    );
  }

  test(
    'an undelivered edit cannot mix sparse chunk boundaries from two revisions',
    () async {
      Debounce.instant = false;
      final dir = await Directory.systemTemp.createTemp(
        'notes-pages-sparse-revisions-',
      );
      final vault = CountingVault((await Vault.open(dir.path)).raw);
      final feed = ChangeFeed(
        vault,
        interval: const Duration(hours: 1),
        wakeDelay: const Duration(hours: 1),
      );
      final store = PagesStore(vault, feed);
      try {
        for (var i = 0; i < 650; i++) {
          await vault.createPage(title: 'Revision $i');
        }
        await vault.flush();
        await feed.start();
        await store.reload();
        final first = await loadedRow(store, 450);
        final second = await loadedRow(store, 430);
        vault.resetCounts();
        vault.listGate = Completer<void>();
        vault.listStarted = Completer<void>();
        var notifications = 0;
        void changed() => notifications++;
        store.addListener(changed);
        await vault.apply(first.id, [edit('delivered first edit')]);
        await feed.poll();
        await vault.listStarted!.future.timeout(const Duration(seconds: 10));
        await vault.apply(second.id, [edit('undelivered second edit')]);
        await vault.flush();
        // No explicit poll here: the refresh must discover this write itself.
        vault.listGate!.complete();
        final wait = Stopwatch()..start();
        while (notifications == 0) {
          if (wait.elapsed > const Duration(seconds: 10)) {
            fail('no current snapshot published');
          }
          await Future<void>.delayed(const Duration(milliseconds: 2));
        }
        store.removeListener(changed);
        expect(notifications, 1);
        expect(vault.pageCountCalls, 0);
        await compareWithFresh(
          vault: vault,
          feed: feed,
          store: store,
          label: 'undelivered sparse edit',
          cov: Coverage(),
        );
      } finally {
        if (vault.listGate != null && !vault.listGate!.isCompleted) {
          vault.listGate!.complete();
        }
        store.dispose();
        feed.stop();
        vault.dispose();
        await dir.delete(recursive: true);
      }
    },
    timeout: const Timeout(Duration(minutes: 5)),
  );

  test(
    'lazy chunks cannot cache an undelivered edit ahead of the store',
    () async {
      Debounce.instant = false;
      final dir = await Directory.systemTemp.createTemp(
        'notes-pages-lazy-revisions-',
      );
      final vault = CountingVault((await Vault.open(dir.path)).raw);
      final feed = ChangeFeed(
        vault,
        interval: const Duration(hours: 1),
        wakeDelay: const Duration(hours: 1),
      );
      final store = PagesStore(vault, feed);
      try {
        for (var i = 0; i < 650; i++) {
          await vault.createPage(title: 'Lazy $i');
        }
        await vault.flush();
        await feed.start();
        await store.reload();
        final edited = await loadedRow(store, 600);
        await vault.apply(edited.id, [edit('undelivered before lazy chunk')]);
        await vault.flush();
        // The new chunk reads the newer index before Dart has heard this batch.
        await loadedRow(store, 450);
        await settle(feed);
        await compareWithFresh(
          vault: vault,
          feed: feed,
          store: store,
          label: 'lazy undelivered edit',
          cov: Coverage(),
        );
      } finally {
        store.dispose();
        feed.stop();
        vault.dispose();
        await dir.delete(recursive: true);
      }
    },
    timeout: const Timeout(Duration(minutes: 5)),
  );

  test('an explicit reload releases metadata from a cancelled content patch', () async {
    Debounce.instant = false;
    final dir = await Directory.systemTemp.createTemp(
      'notes-pages-reload-meta-',
    );
    final vault = CountingVault((await Vault.open(dir.path)).raw);
    final feed = ChangeFeed(
      vault,
      interval: const Duration(hours: 1),
      wakeDelay: const Duration(hours: 1),
    );
    final store = PagesStore(vault, feed);
    try {
      final id = await vault.createPage();
      final block = edit('Old first line');
      await vault.apply(id, [block]);
      await vault.flush();
      await feed.start();
      await store.reload();
      store.adopt(await vault.loadPage(id));
      vault.listGate = Completer<void>();
      vault.listStarted = Completer<void>();
      await vault.apply(id, [
        BOp(
          op: OpKind.setText,
          id: block.id,
          kind: Kinds.paragraph,
          text: 'New first line',
          marks: const [],
          checked: false,
        ),
      ]);
      await feed.poll();
      await vault.listStarted!.future.timeout(const Duration(seconds: 10));
      // The prefix response is still held while the full reload supersedes it.
      await store.reload();
      final metadata = store.meta(id);
      if (metadata != null) {
        expect(
          metadata.firstLine,
          'New first line',
          reason: 'the cancelled patch must not keep metadata stale',
        );
      }
      await settle(feed);
      expect(store.meta(id)!.firstLine, 'New first line');
      vault.listGate!.complete();
      await settle(feed);
      expect(store.meta(id)!.firstLine, 'New first line');
      expect(store.rowAt(0)!.id, id);
    } finally {
      if (vault.listGate != null && !vault.listGate!.isCompleted) {
        vault.listGate!.complete();
      }
      store.dispose();
      feed.stop();
      vault.dispose();
      await dir.delete(recursive: true);
    }
  }, timeout: const Timeout(Duration(minutes: 2)));

  test(
    'concurrent explicit reloads share a retry after a changed generation',
    () async {
      Debounce.instant = false;
      final dir = await Directory.systemTemp.createTemp(
        'notes-pages-reload-shared-',
      );
      final vault = CountingVault((await Vault.open(dir.path)).raw);
      final feed = ChangeFeed(
        vault,
        interval: const Duration(hours: 1),
        wakeDelay: const Duration(hours: 1),
      );
      final store = PagesStore(vault, feed);
      try {
        final id = await vault.createPage(title: 'Target');
        await vault.createPage(title: 'Newest');
        await vault.flush();
        await feed.start();
        await store.reload();
        vault.resetCounts();
        vault.countGate = Completer<void>();
        vault.countStarted = Completer<void>();
        final first = store.reload();
        final second = store.reload();
        await vault.countStarted!.future.timeout(const Duration(seconds: 10));
        await vault.renamePage(id, 'Renamed target');
        await feed.poll();
        vault.countGate!.complete();
        await Future.wait([first, second]);
        expect(
          vault.pageCountCalls,
          2,
          reason: 'one cancelled attempt and one retry',
        );
        expect(store.rowAt(0)!.id, id);
        expect(store.docAt(0)!.title, 'Renamed target');
      } finally {
        if (vault.countGate != null && !vault.countGate!.isCompleted) {
          vault.countGate!.complete();
        }
        store.dispose();
        feed.stop();
        vault.dispose();
        await dir.delete(recursive: true);
      }
    },
    timeout: const Timeout(Duration(minutes: 2)),
  );

  test(
    'public reload stops after five superseded rounds and defers one full reload',
    () async {
      Debounce.instant = false;
      const rounds = 5;
      final dir = await Directory.systemTemp.createTemp(
        'notes-pages-reload-cap-',
      );
      final vault = CountingVault((await Vault.open(dir.path)).raw);
      final feed = ChangeFeed(
        vault,
        interval: const Duration(hours: 1),
        wakeDelay: const Duration(hours: 1),
      );
      final store = PagesStore(vault, feed);
      var injecting = false;
      // Holds a sixth first-chunk response while injection is on: without the
      // cap that round would never end, so the test stops writing and waits.
      final held = Completer<void>();
      Future<Object?>? reloads;
      try {
        for (var i = 0; i < 30; i++) {
          await vault.createPage(title: 'Capped $i');
        }
        await vault.flush();
        await feed.start();
        await store.reload();
        await settle(feed);
        // The old safe state, read before any injected write. Reads only the
        // loaded chunk, so it makes no vault calls.
        final oldCount = store.count;
        final oldRows = await allRows(store);
        final oldDocs = await allDocs(store, feed);
        vault.resetCounts();
        var injected = 0;
        var chunkResponses = 0;
        // Requests for the first chunk (offset 0, limit 100), one per round.
        int firstChunkCalls() =>
            vault.requests.where((r) => r.offset == 0 && r.limit == 100).length;
        // The first five rounds are each superseded by a real structural write
        // (before their count and before their first chunk returns). A sixth
        // round is held while injection is on, so a pre-fix loop cannot run on.
        vault.onResponse = (call, offset, limit) async {
          if (!injecting) return;
          if (call == 'pageCount') {
            if (chunkResponses < rounds) {
              injected++;
              await vault.createPage(title: 'Injected $injected');
            }
            return;
          }
          if (offset != 0 || limit != 100) return;
          chunkResponses++;
          if (chunkResponses > rounds) {
            await held.future;
            return;
          }
          injected++;
          await vault.createPage(title: 'Injected $injected');
        };
        injecting = true;
        // Two concurrent callers share one request, so they run one set of rounds.
        final first = store.reload();
        final second = store.reload();
        reloads = Future.wait([first, second]);
        await reloads.timeout(const Duration(seconds: 20));
        // No round committed, so the old safe state is still what the store shows
        // at once: same count, same loaded rows, same documents.
        expect(store.count, oldCount, reason: 'old count stays after the cap');
        for (var i = 0; i < oldRows.length; i++) {
          final row = store.rowAt(i);
          expect(row, isNotNull, reason: 'old row $i stays loaded');
          expectSameRow(row!, oldRows[i], 'capped old row $i');
        }
        expect(
          store.docCount,
          oldDocs.length,
          reason: 'old document count stays after the cap',
        );
        for (var i = 0; i < oldDocs.length; i++) {
          final doc = store.docAt(i);
          expect(doc, isNotNull, reason: 'old document $i stays loaded');
          expectSameRow(doc!, oldDocs[i], 'capped old document $i');
        }
        expect(
          vault.pageCountCalls,
          rounds,
          reason: 'five rounds, then the cap',
        );
        expect(firstChunkCalls(), rounds, reason: 'five first-chunk requests');
        expect(
          Debounce.pending,
          1,
          reason: 'one deferred full reload is scheduled',
        );
        // No sixth round starts right after the public future completes.
        await Future<void>.delayed(const Duration(milliseconds: 50));
        expect(vault.pageCountCalls, rounds, reason: 'no immediate sixth round');
        expect(firstChunkCalls(), rounds, reason: 'no immediate sixth round');
        // Writes stop. The single deferred reload runs once, then settles.
        injecting = false;
        await settle(feed);
        expect(
          vault.pageCountCalls,
          rounds + 1,
          reason: 'exactly one deferred full reload',
        );
        expect(
          firstChunkCalls(),
          rounds + 1,
          reason: 'exactly one deferred first-chunk request',
        );
        // The fresh store's own calls come only after the call assertions.
        vault.onResponse = null;
        await compareWithFresh(
          vault: vault,
          feed: feed,
          store: store,
          label: 'capped reload',
          cov: Coverage(),
        );
      } finally {
        injecting = false;
        vault.onResponse = null;
        // Release a held round before draining, so a hung reload can finish.
        if (!held.isCompleted) held.complete();
        if (reloads != null) {
          // Drain a hung reload once injection stops, so its rounds end cleanly.
          try {
            await reloads.timeout(const Duration(seconds: 30));
          } catch (_) {}
        }
        store.dispose();
        feed.stop();
        vault.dispose();
        await dir.delete(recursive: true);
      }
    },
    timeout: const Timeout(Duration(minutes: 2)),
  );

  for (final renamed in [true, false]) {
    test(
      renamed
          ? 'an awaited reload right after a rename returns the renamed page first'
          : 'an awaited reload right after a content edit returns the edited page first',
      () async {
        Debounce.instant = false;
        final dir = await Directory.systemTemp.createTemp(
          'notes-pages-reload-first-',
        );
        final vault = CountingVault((await Vault.open(dir.path)).raw);
        final feed = ChangeFeed(
          vault,
          interval: const Duration(hours: 1),
          wakeDelay: const Duration(hours: 1),
        );
        final store = PagesStore(vault, feed);
        try {
          // Created first, so it starts below the fillers and outside the first six.
          final target = await vault.createPage(title: 'Target');
          for (var i = 0; i < 30; i++) {
            await vault.createPage(title: 'Filler $i');
          }
          await vault.flush();
          await feed.start();
          await store.reload();
          await settle(feed);
          final before = await allRows(store);
          expect(
            before.indexWhere((r) => r.id == target),
            greaterThanOrEqualTo(6),
            reason: 'the target starts outside the first six rows',
          );
          if (renamed) {
            await vault.renamePage(target, 'Target renamed');
          } else {
            await vault.apply(target, [edit('edited first line')]);
          }
          // Nothing is polled or settled after this await: the reload's own
          // return must leave the store fresh.
          await store.reload();
          final title = renamed ? 'Target renamed' : 'Target';
          expect(store.count, before.length, reason: 'count is fresh');
          for (var i = 0; i < 6; i++) {
            expect(store.rowAt(i), isNotNull, reason: 'row $i is loaded');
          }
          expect(store.rowAt(0)!.id, target, reason: 'target is first');
          expect(store.rowAt(0)!.title, title, reason: 'first row is fresh');
          expect(store.docAt(0), isNotNull, reason: 'document 0 is loaded');
          expect(
            store.docAt(0)!.id,
            target,
            reason: 'target is first document',
          );
          expect(
            store.docAt(0)!.title,
            title,
            reason: 'first document is fresh',
          );
          await compareWithFresh(
            vault: vault,
            feed: feed,
            store: store,
            label: renamed
                ? 'reload after rename'
                : 'reload after content edit',
            cov: Coverage(),
          );
        } finally {
          store.dispose();
          feed.stop();
          vault.dispose();
          await dir.delete(recursive: true);
        }
      },
      timeout: const Timeout(Duration(minutes: 2)),
    );
  }

  for (final seed in seeds) {
    test('PagesStore matches a fresh store after every window (seed $seed)', () async {
      Debounce.instant = false;
      final dir = await Directory.systemTemp.createTemp(
        'notes-pages-incremental-',
      );
      final vault = CountingVault((await Vault.open(dir.path)).raw);
      final feed = ChangeFeed(
        vault,
        interval: const Duration(hours: 1),
        wakeDelay: const Duration(hours: 1),
      );
      final store = PagesStore(vault, feed);
      final rng = math.Random(seed);
      final cov = Coverage();
      final ids = <String, String>{};
      String id(String name) => ids[name]!;

      var windowBatches = 0;
      var windowTree = 0;
      var notes = 0;
      final changed = <String>{};
      final windowKinds = <String>[];
      // Set by any window step that writes the hidden calendars page.
      var hiddenWrote = false;
      void onBatch(ChangeBatch b) {
        windowBatches++;
        if (b.treeChanged) windowTree++;
        if (b.reset) windowKinds.add('reset');
        for (final c in b.changes) {
          windowKinds.add(c.kind.name);
          if (c.page != null) changed.add(c.page!);
        }
      }

      void onStore() => notes++;

      // The fixed script. Each window is one op or one burst; names refer to
      // seeded pages, so every step is deterministic.
      final script = <Future<String> Function()>[
        () async {
          await vault.apply(id('Doc 3'), [edit('content 1')]);
          return 'content Doc 3';
        },
        () async {
          await vault.apply(id('Doc 4'), [edit('burst 1')]);
          await vault.apply(id('Row 0.0'), [edit('burst 2')]);
          await vault.apply(id('Nested 2'), [edit('burst 3')]);
          return 'content burst x3';
        },
        () async {
          await vault.renamePage(id('Doc 5'), 'Doc 5 renamed');
          return 'rename Doc 5';
        },
        () async {
          ids['Created A'] = await vault.createPage(title: 'Created A');
          return 'create top-level Created A';
        },
        () async {
          ids['Row 1.new'] = await vault.createPage(
            parent: id('Database 1'),
            title: 'Row 1.new',
          );
          return 'create row Row 1.new';
        },
        () async {
          await vault.deletePage(id('Row 0.3'));
          return 'delete Row 0.3';
        },
        () async {
          await vault.movePage(id('Nested 1'), newParent: id('Doc 7'));
          return 'move Nested 1 under Doc 7';
        },
        () async {
          await vault.setPageType(id('Doc 9'), databaseType);
          return 'document to database Doc 9';
        },
        () async {
          await vault.setPageType(id('Database 2'), documentType);
          return 'database to document Database 2';
        },
        () async {
          await vault.apply(id('Row 1.new'), [edit('row content')]);
          return 'content Row 1.new';
        },
        () async {
          await vault.renamePage(id('Row 0.4'), 'Row 0.4 renamed');
          return 'rename Row 0.4';
        },
        () async {
          await vault.movePage(id('Row 3.5'));
          return 'move Row 3.5 to top level';
        },
        () async {
          await vault.deletePage(id('Database 3'));
          return 'delete Database 3 (with rows)';
        },
        () async {
          await vault.setPageType(id('Doc 12'), databaseType);
          return 'document to database Doc 12';
        },
        () async {
          ids['Row 12.new'] = await vault.createPage(
            parent: id('Doc 12'),
            title: 'Row 12.new',
          );
          return 'create row Row 12.new under Doc 12';
        },
        () async {
          await vault.apply(id('Doc 9'), [edit('content in database')]);
          return 'content Doc 9';
        },
        // Hidden-only calendar writes: the store must reload and still exclude it.
        () async {
          hiddenWrote = true;
          await vault.createCalendar(
            name: 'Hidden only',
            color: 'sky',
            showInCalendar: true,
            showInToday: true,
          );
          return 'hidden create calendar';
        },
        () async {
          hiddenWrote = true;
          final c = (await vault.listCalendars()).calendars.first;
          await vault.updateCalendar(
            CalendarCategory(
              id: c.id,
              name: 'Personal renamed',
              color: c.color,
              showInCalendar: !c.showInCalendar,
              showInToday: c.showInToday,
            ),
          );
          return 'hidden update first calendar';
        },
        () async {
          hiddenWrote = true;
          final cats = (await vault.listCalendars()).calendars;
          await vault.reorderCalendars([for (final c in cats.reversed) c.id]);
          return 'hidden reverse calendars';
        },
        () async {
          hiddenWrote = true;
          final cats = (await vault.listCalendars()).calendars;
          await vault.deleteCalendar(cats.last.id);
          return 'hidden delete last calendar';
        },
        () async {
          hiddenWrote = true;
          await vault.setDatabaseCalendarVisibility(
            id('Database 0'),
            showInCalendar: false,
          );
          return 'hidden database 0 off calendar';
        },
        // Mixed windows: a visible write and a hidden write in one batch.
        () async {
          hiddenWrote = true;
          await vault.createCalendar(
            name: 'Mixed',
            color: 'duvet',
            showInCalendar: true,
            showInToday: false,
          );
          await vault.apply(id('Doc 13'), [edit('mixed content with calendar')]);
          return 'mixed create calendar and content Doc 13';
        },
        () async {
          hiddenWrote = true;
          final c = (await vault.listCalendars()).calendars.first;
          await vault.updateCalendar(
            CalendarCategory(
              id: c.id,
              name: c.name,
              color: c.color,
              showInCalendar: c.showInCalendar,
              showInToday: !c.showInToday,
            ),
          );
          await vault.renamePage(id('Doc 14'), 'Doc 14 renamed');
          return 'mixed update calendar and rename Doc 14';
        },
      ];

      /// One hidden-page write: a category create, update, delete or reorder, or
      /// a database's calendar visibility. Always writes (never skipped).
      Future<String> randomCalendars(List<PageRow> model) async {
        final cats = (await vault.listCalendars()).calendars;
        final dbs = [for (final p in model) if (p.pageType == databaseType) p];
        var op = rng.nextInt(5);
        if ((op != 4 && cats.isEmpty) || (op == 4 && dbs.isEmpty)) op = 0;
        hiddenWrote = true;
        if (op == 0) {
          await vault.createCalendar(
            name: 'random calendar ${rng.nextInt(1000)}',
            color: calendarColors[rng.nextInt(calendarColors.length)],
            showInCalendar: rng.nextBool(),
            showInToday: rng.nextBool(),
          );
          return 'random create calendar';
        }
        if (op == 1) {
          final c = cats[rng.nextInt(cats.length)];
          await vault.updateCalendar(
            CalendarCategory(
              id: c.id,
              name: 'Renamed ${rng.nextInt(100)}',
              color: c.color,
              showInCalendar: !c.showInCalendar,
              showInToday: c.showInToday,
            ),
          );
          return 'random update calendar';
        }
        if (op == 2) {
          await vault.deleteCalendar(cats[rng.nextInt(cats.length)].id);
          return 'random delete calendar';
        }
        if (op == 3) {
          final order = [for (final c in cats) c.id]..shuffle(rng);
          await vault.reorderCalendars(order);
          return 'random reorder calendars';
        }
        final db = dbs[rng.nextInt(dbs.length)];
        await vault.setDatabaseCalendarVisibility(
          db.id,
          showInCalendar: rng.nextBool(),
        );
        return 'random database calendar visibility';
      }

      Future<String> randomWindow(List<PageRow> model) async {
        final byId = {for (final p in model) p.id: p};
        final touched = <String>{};
        PageRow? pick([bool Function(PageRow)? where]) {
          final c = [
            for (final p in model)
              if (!touched.contains(p.id) && (where == null || where(p))) p,
          ];
          if (c.isEmpty) return null;
          final p = c[rng.nextInt(c.length)];
          touched.add(p.id);
          return p;
        }

        bool wouldCycle(PageRow p, PageRow? target) {
          var a = target;
          while (a != null) {
            if (a.id == p.id) return true;
            a = a.parent == null ? null : byId[a.parent];
          }
          return false;
        }

        if (rng.nextInt(5) == 0) {
          final desc = await randomCalendars(model);
          // Half the time, a visible content write in the same window.
          if (model.isNotEmpty && rng.nextBool()) {
            final p = model[rng.nextInt(model.length)];
            await vault.apply(p.id, [edit('mixed random content')]);
            return '$desc with content';
          }
          return desc;
        }
        if (rng.nextInt(4) == 0) {
          var k = 0;
          for (; k < 3; k++) {
            final p = pick();
            if (p == null) break;
            await vault.apply(p.id, [edit('random burst $k')]);
          }
          return k == 0 ? 'skipped random burst' : 'random content burst x$k';
        }
        final kind = opKinds[rng.nextInt(opKinds.length)];
        if (kind == 'content') {
          final p = pick();
          if (p == null) return 'skipped random content';
          await vault.apply(p.id, [edit('random content')]);
        } else if (kind == 'rename') {
          final p = pick();
          if (p == null) return 'skipped random rename';
          await vault.renamePage(p.id, 'random renamed');
        } else if (kind == 'create') {
          final parent = rng.nextInt(4) == 0 ? null : pick();
          await vault.createPage(parent: parent?.id, title: 'random new');
        } else if (kind == 'delete') {
          final p = pick();
          if (p == null) return 'skipped random delete';
          await vault.deletePage(p.id);
        } else if (kind == 'move') {
          final p = pick();
          if (p == null) return 'skipped random move';
          final target = rng.nextInt(4) == 0
              ? null
              : model[rng.nextInt(model.length)];
          if (wouldCycle(p, target)) return 'skipped random move (cycle)';
          await vault.movePage(p.id, newParent: target?.id);
        } else if (kind == 'toDatabase') {
          final p = pick((x) => x.pageType == documentType);
          if (p == null) return 'skipped random toDatabase';
          await vault.setPageType(p.id, databaseType);
        } else {
          final p = pick((x) => x.pageType == databaseType);
          if (p == null) return 'skipped random toDocument';
          await vault.setPageType(p.id, documentType);
        }
        return 'random $kind';
      }

      try {
        // Seed before the feed starts: nothing is delivered for the seeding,
        // and the first reload reads the finished vault.
        await seedVault(vault, rng, ids);
        // Created before the feed starts, so the hidden page is part of the seed.
        await vault.listCalendars();
        await vault.flush();
        final hidden = await hiddenCalendarsPage(vault);
        await feed.start();
        await store.reload();
        await settle(feed);
        feed.addListener(onBatch);
        store.addListener(onStore);
        await compareWithFresh(
          vault: vault,
          feed: feed,
          store: store,
          label: 'seed $seed initial',
          cov: cov,
          hidden: hidden,
        );
        await expectCoreExcludes(
          vault: vault,
          store: store,
          hidden: hidden,
          label: 'seed $seed initial',
        );

        final total = script.length + randomWindows;
        for (var w = 0; w < total; w++) {
          await settle(feed);
          // Listed before the counters reset, so the test's own listing is
          // not part of the window.
          final model = await vault.listPages(offset: 0, limit: 100000);
          vault.resetCounts();
          windowBatches = 0;
          windowTree = 0;
          notes = 0;
          changed.clear();
          windowKinds.clear();
          hiddenWrote = false;

          final desc = w < script.length
              ? await script[w]()
              : await randomWindow(model);
          await settle(feed);
          final wrote = !desc.startsWith('skipped');
          final hiddenWindow = hiddenWrote;
          final label = 'seed $seed window $w ($desc)';

          // ignore: avoid_print
          print(
            'CHANGE_KINDS $label hidden=$hiddenWindow batches=$windowBatches tree=$windowTree '
            'changed=${changed.length} '
            'kinds=${(windowKinds.toSet().toList()..sort())} '
            'listPages=[${vault.requests.join('; ')}] '
            'pageCount=${vault.pageCountCalls} notifications=$notes',
          );

          // Hidden-page windows are checked for correctness only: the store
          // reloads, so the in-place cost rules do not apply to them.
          if (hiddenWindow) {
            cov.hidden++;
          } else if (windowTree == 0) {
            cov.ordinary++;
            if (wrote) {
              // No rewalk first: a full reload requests offset 0, limit 100.
              for (final r in vault.requests) {
                expect(
                  r.offset,
                  0,
                  reason:
                      '$label: an in-place edit must not page past the first row',
                );
                expect(
                  r.limit,
                  lessThanOrEqualTo(changed.length),
                  reason:
                      '$label: an in-place edit must not request more rows than it changed',
                );
              }
              expect(
                vault.requests.fold<int>(0, (n, r) => n + r.returned),
                lessThanOrEqualTo(changed.length),
                reason: '$label: changed rows only',
              );
              expect(
                vault.pageCountCalls,
                0,
                reason: '$label: an in-place edit must not count pages',
              );
              expect(
                windowBatches,
                1,
                reason: '$label: one write window is one batch',
              );
              expect(
                notes,
                windowBatches,
                reason: '$label: at most one notification per batch',
              );
            }
          } else {
            cov.tree++;
          }

          await compareWithFresh(
            vault: vault,
            feed: feed,
            store: store,
            label: label,
            cov: cov,
            hidden: hidden,
          );
          await expectCoreExcludes(
            vault: vault,
            store: store,
            hidden: hidden,
            label: label,
          );
        }

        // ignore: avoid_print
        print('PAGES_STORE_INCREMENTAL seed=$seed $cov');
        expect(
          cov.ordinary,
          greaterThan(0),
          reason: 'the run must include in-place windows',
        );
        expect(
          cov.tree,
          greaterThan(0),
          reason: 'the run must include tree-changing windows',
        );
        expect(
          cov.hidden,
          greaterThan(0),
          reason: 'the run must include hidden calendars windows',
        );
      } finally {
        feed.removeListener(onBatch);
        store.removeListener(onStore);
        store.dispose();
        feed.stop();
        vault.dispose();
        try {
          await dir.delete(recursive: true);
        } catch (_) {}
      }
    }, timeout: const Timeout(Duration(minutes: 15)));
  }
}
