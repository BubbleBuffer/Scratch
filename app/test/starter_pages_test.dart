import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/features/getting_started.dart';
import 'package:notes_app/state/sync_store.dart';

import 'harness.dart' show initCore, resetTestClock;

Future<(Directory, Vault)> _emptyVault() async {
  final dir = await Directory.systemTemp.createTemp('notes_starter_pages_test_');
  final vault = await Vault.open('${dir.path}${Platform.pathSeparator}vault');
  return (dir, vault);
}

Future<List<String>> _rootTitles(Vault vault) async => [for (final p in await vault.listChildren(null)) p.title];

/// A SyncStore whose status the test publishes, as the background loop would.
class _ProbeSync extends SyncStore {
  _ProbeSync(super.vault);

  void publish(SyncStatus next) {
    status = next;
    notifyListeners();
  }
}

void main() {
  setUpAll(initCore);

  // Every fresh vault already holds its device's node record (a child of the
  // hidden nodes page). The gate must treat that as no pages, so each test
  // below seeds on an otherwise empty vault.

  test('a first pull that brings pages gives no starter pages', () async {
    resetTestClock();
    final (dir, vault) = await _emptyVault();
    try {
      // The pull has already written the hub's pages into the vault.
      await vault.createPage(title: 'From the hub');
      await vault.flush();
      final seeded = await seedStarterPagesAfterPull(vault, Future.value(true));
      expect(seeded, isFalse);
      expect(await _rootTitles(vault), ['From the hub']);
    } finally {
      vault.dispose();
      await dir.delete(recursive: true);
    }
  });

  test('a first pull that brings nothing gives starter pages once', () async {
    resetTestClock();
    final (dir, vault) = await _emptyVault();
    try {
      expect(await seedStarterPagesAfterPull(vault, Future.value(true)), isTrue);
      expect(await _rootTitles(vault), [gettingStartedTitle, 'Tasks', 'Calendar']);
      // A later call finds the pages it made and adds nothing.
      expect(await seedStarterPagesAfterPull(vault, Future.value(true)), isFalse);
      expect(await _rootTitles(vault), [gettingStartedTitle, 'Tasks', 'Calendar']);
    } finally {
      vault.dispose();
      await dir.delete(recursive: true);
    }
  });

  test('a first pull that fails gives no starter pages this launch', () async {
    resetTestClock();
    final (dir, vault) = await _emptyVault();
    try {
      expect(await seedStarterPagesAfterPull(vault, Future.value(false)), isFalse);
      expect(await _rootTitles(vault), isEmpty);
    } finally {
      vault.dispose();
      await dir.delete(recursive: true);
    }
  });

  test('a first pull that completes later waits for it before seeding', () async {
    resetTestClock();
    final (dir, vault) = await _emptyVault();
    try {
      final pull = Completer<bool>();
      final seeding = seedStarterPagesAfterPull(vault, pull.future);
      await vault.flush();
      expect(await _rootTitles(vault), isEmpty);
      pull.complete(true);
      expect(await seeding, isTrue);
      expect(await _rootTitles(vault), [gettingStartedTitle, 'Tasks', 'Calendar']);
    } finally {
      vault.dispose();
      await dir.delete(recursive: true);
    }
  });

  test('without sync, starter pages are created at boot', () async {
    resetTestClock();
    final (dir, vault) = await _emptyVault();
    try {
      expect(await seedStarterPagesAtBoot(vault, syncing: false), isTrue);
      expect(await _rootTitles(vault), [gettingStartedTitle, 'Tasks', 'Calendar']);
    } finally {
      vault.dispose();
      await dir.delete(recursive: true);
    }
  });

  test('firstSyncOk completes at once when a sync round has already succeeded', () async {
    resetTestClock();
    final (dir, vault) = await _emptyVault();
    final sync = SyncStore(vault);
    try {
      sync.status = const SyncStatus(running: true, syncing: false, syncs: 1, lastOkMs: 1);
      expect(await sync.firstSyncOk(), isTrue);
    } finally {
      sync.dispose();
      vault.dispose();
      await dir.delete(recursive: true);
    }
  });

  test('firstSyncOk waits for the first successful round, then completes true', () async {
    resetTestClock();
    final (dir, vault) = await _emptyVault();
    final sync = _ProbeSync(vault);
    try {
      final first = sync.firstSyncOk();
      sync.publish(const SyncStatus(running: true, syncing: false, syncs: 0, lastError: 'hub connection'));
      var done = false;
      unawaited(first.then((_) => done = true));
      await Future<void>.delayed(Duration.zero);
      expect(done, isFalse);
      sync.publish(const SyncStatus(running: true, syncing: false, syncs: 1, lastOkMs: 1));
      expect(await first, isTrue);
    } finally {
      sync.dispose();
      vault.dispose();
      await dir.delete(recursive: true);
    }
  });

  test('firstSyncOk completes false when the vault closes before a round succeeds', () async {
    resetTestClock();
    final (dir, vault) = await _emptyVault();
    final sync = _ProbeSync(vault);
    try {
      final first = sync.firstSyncOk();
      vault.dispose();
      sync.publish(const SyncStatus(running: false, syncing: false, syncs: 0));
      expect(await first, isFalse);
    } finally {
      sync.dispose();
      await dir.delete(recursive: true);
    }
  });

  test('with sync configured, boot creates no starter pages', () async {
    resetTestClock();
    final (dir, vault) = await _emptyVault();
    try {
      expect(await seedStarterPagesAtBoot(vault, syncing: true), isFalse);
      expect(await _rootTitles(vault), isEmpty);
    } finally {
      vault.dispose();
      await dir.delete(recursive: true);
    }
  });
}
