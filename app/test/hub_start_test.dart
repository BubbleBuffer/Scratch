import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/features/sync/sync_control.dart';
import 'package:notes_app/state/sync_store.dart';

import 'harness.dart' show initCore;

void main() {
  setUpAll(initCore);

  late Directory dir;
  late Vault vault;
  late SyncStore sync;
  late AppSettings settings;

  setUp(() async {
    debugDefaultTargetPlatformOverride = TargetPlatform.linux;
    dir = Directory.systemTemp.createTempSync('notes-hub-start-');
    vault = await Vault.open('${dir.path}/vault', dataDir: '${dir.path}/data');
    sync = SyncStore(vault);
    settings = AppSettings(file: File('${dir.path}/settings.json'));
  });

  tearDown(() async {
    await sync.stopHub();
    sync.dispose();
    vault.dispose();
    settings.dispose();
    debugDefaultTargetPlatformOverride = null;
    dir.deleteSync(recursive: true);
  });

  test('failed hub start exposes the error and saves the switch off', () async {
    settings.update((s) {
      s.hubEnabled = false;
      s.hubAddress = 'invalid-address';
    });

    await setHubEnabled(sync, settings, true);

    expect(sync.hosting, isFalse);
    expect(sync.hubAddress, isNull);
    expect(sync.hubError, contains('Could not start the hub'));
    expect(sync.phase, (SyncPhase.error, sync.hubError));
    expect(settings.hubEnabled, isFalse);
    expect(AppSettings.load(settings.file!).hubEnabled, isFalse);
  });

  test(
    'failed address restart turns sharing off; retry clears the error',
    () async {
      settings.update((s) => s.hubAddress = '127.0.0.1:0');
      await setHubEnabled(sync, settings, true);
      expect(sync.hosting, isTrue);
      expect(settings.hubEnabled, isTrue);

      await setHubAddress(sync, settings, 'invalid-address');

      expect(sync.hosting, isFalse);
      expect(settings.hubEnabled, isFalse);
      expect(sync.hubError, isNotNull);
      expect(AppSettings.load(settings.file!).hubEnabled, isFalse);

      await setHubAddress(sync, settings, '127.0.0.1:0');
      await setHubEnabled(sync, settings, true);

      expect(sync.hosting, isTrue);
      expect(settings.hubEnabled, isTrue);
      expect(sync.hubError, isNull);

      await setHubEnabled(sync, settings, false);
      expect(sync.hosting, isFalse);
      expect(settings.hubEnabled, isFalse);
    },
  );
}
