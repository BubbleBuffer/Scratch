// A demo-sized Today fixture on the real core, shared by tests and goldens.

import 'dart:io';

import 'package:notes_app/core/vault.dart';
import 'package:notes_app/features/demo.dart';
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/features/reminders/backends.dart';

import 'harness.dart' show resetTestClock;

class TodayFixture {
  TodayFixture(this.dir, this.app, this.pages);
  final Directory dir;
  final AppState app;
  final Map<String, String> pages;
  Vault get vault => app.vault;

  Future<void> dispose() async {
    await app.dispose();
    // Bridge work already in flight owns the final native handle briefly on
    // Windows. Retry only its sharing violation; other cleanup errors fail.
    for (var attempt = 0; ; attempt++) {
      try {
        await dir.delete(recursive: true);
        return;
      } on FileSystemException catch (error) {
        if (!Platform.isWindows ||
            error.osError?.errorCode != 32 ||
            attempt >= 19) {
          rethrow;
        }
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
    }
  }
}

Future<TodayFixture> createTodayFixture({bool empty = false}) async {
  resetTestClock();
  final dir = await Directory.systemTemp.createTemp('notes_today_test_');
  final vault = await Vault.open('${dir.path}${Platform.pathSeparator}vault');
  final pages = empty ? <String, String>{} : await createDemo(vault);
  await vault.run((v) => v.ensureSpaceColors(), writes: true);
  await vault.flush();
  final app = AppState(vault, reminderBackend: FakeReminderBackend());
  await app.start();
  return TodayFixture(dir, app, pages);
}
