import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/change_feed.dart';
import 'package:notes_app/core/debounce.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/features/reminders/backends.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/state/agenda_store.dart';
import 'package:notes_app/state/collection_store.dart';
import 'package:notes_app/state/page_store.dart';
import 'package:notes_app/state/pages_store.dart';
import 'package:notes_app/state/reminder_store.dart';
import 'package:notes_app/state/sync_store.dart';
import 'package:notes_app/state/tree_store.dart';

import 'harness.dart' show initCore;

class ObservedVault extends Vault {
  ObservedVault(super.raw);

  bool closed = false;
  int callsAfterClose = 0;
  Completer<void>? countGate;
  final countStarted = Completer<void>();

  @override
  Future<T> run<T>(Future<T> Function(NotesVault v) f, {bool writes = false}) {
    if (closed) callsAfterClose++;
    return super.run(f, writes: writes);
  }

  @override
  Future<int> pageCount() async {
    final count = await super.pageCount();
    if (!countStarted.isCompleted) countStarted.complete();
    await countGate?.future;
    return count;
  }

  @override
  void dispose() {
    super.dispose();
    closed = true;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initCore);

  test('closing the vault cancels a pending PagesStore reload', () async {
    final dir = await Directory.systemTemp.createTemp('notes-close-reload-');
    final vault = ObservedVault((await Vault.open(dir.path)).raw);
    final feed = ChangeFeed(vault, interval: const Duration(hours: 1));
    final pages = PagesStore(vault, feed);
    final oldInstant = Debounce.instant;
    Debounce.instant = false;
    try {
      await feed.start();
      await vault.createPage(title: 'Pending reload');
      await feed.poll();
      expect(
        Debounce.pending,
        1,
        reason: 'the change batch scheduled the reload',
      );
      // Leave the store alive: closing the vault itself must cancel the work.
      vault.dispose();
      expect(
        Debounce.pending,
        0,
        reason: 'close cancels the timer synchronously',
      );
      expect(
        feed.idle,
        isTrue,
        reason: 'the pending write wake is cancelled too',
      );
      await Future<void>.delayed(
        PagesStore.reloadDelay + const Duration(milliseconds: 50),
      );
      expect(vault.callsAfterClose, 0);
      expect(Debounce.pending, 0);
    } finally {
      pages.dispose();
      feed.stop();
      if (!vault.closed) vault.dispose();
      Debounce.instant = oldInstant;
      await dir.delete(recursive: true);
    }
  });

  test(
    'a PagesStore reload stops between bridge calls when the vault closes',
    () async {
      final dir = await Directory.systemTemp.createTemp(
        'notes-close-inflight-',
      );
      final vault = ObservedVault((await Vault.open(dir.path)).raw);
      final feed = ChangeFeed(vault);
      final pages = PagesStore(vault, feed);
      vault.countGate = Completer<void>();
      try {
        final reload = pages.reload();
        await vault.countStarted.future;
        vault.dispose();
        vault.countGate!.complete();
        await reload;
        expect(vault.callsAfterClose, 0);
      } finally {
        pages.dispose();
        feed.stop();
        if (!vault.closed) vault.dispose();
        await dir.delete(recursive: true);
      }
    },
  );

  test(
    'vault close cancels every store debounce and deferred page check',
    () async {
      final dir = await Directory.systemTemp.createTemp('notes-close-stores-');
      final vault = ObservedVault((await Vault.open(dir.path)).raw);
      final feed = ChangeFeed(
        vault,
        interval: const Duration(hours: 1),
        wakeDelay: const Duration(hours: 1),
      );
      final pages = PagesStore(vault, feed);
      final tree = TreeStore(vault, feed);
      final agenda = AgendaStore(vault, feed);
      final sync = SyncStore(vault, feed);
      final pageStore = PageStore(vault, feed);
      final settings = AppSettings();
      final reminders = ReminderStore(
        vault,
        feed,
        settings,
        backend: FakeReminderBackend(),
      );
      final oldInstant = Debounce.instant;
      Debounce.instant = false;
      CollectionStore? collection;
      try {
        final id = await vault.createPage(title: 'Before');
        collection = CollectionStore(vault, feed, id, withRows: false);
        await collection.load();
        tree.childrenOf(null);
        final wait = Stopwatch()..start();
        while (tree.loadedChildren(null) == null) {
          if (wait.elapsed > const Duration(seconds: 5)) {
            fail('tree did not load');
          }
          await Future<void>.delayed(const Duration(milliseconds: 5));
        }
        await reminders.start();
        await feed.start();
        final session = pageStore.open(id);
        session.rename('After');
        await session.save();
        await feed.poll();
        // Pages, tree, collection, agenda, sync and reminders each scheduled
        // a debounce. PageSession also scheduled its own 1500 ms echo check.
        expect(Debounce.pending, 6);
        vault.dispose();
        expect(Debounce.pending, 0);
        await Future<void>.delayed(const Duration(milliseconds: 2100));
        expect(vault.callsAfterClose, 0);
        // Closing is terminal even if something tries to restart the feed.
        await feed.start();
        feed.wake();
        await feed.poll();
        expect(vault.callsAfterClose, 0);
        expect(feed.idle, isTrue);
      } finally {
        collection?.dispose();
        pages.dispose();
        tree.dispose();
        agenda.dispose();
        sync.dispose();
        pageStore.dispose();
        await reminders.close();
        settings.dispose();
        feed.stop();
        if (!vault.closed) vault.dispose();
        Debounce.instant = oldInstant;
        await dir.delete(recursive: true);
      }
    },
  );
}
