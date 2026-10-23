// Where the app keeps its data, and a small append-only app log.

import 'dart:io';

import '../feedback/log_buffer.dart';
import 'clock.dart';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// The app's data folder.
/// - `NOTES_DATA_DIR` (environment) overrides it on desktop (start checks,
///   tests).
/// - Windows: %LOCALAPPDATA%\Notes (kept short because of the 260-character
///   path limit).
/// - Android: the app's files dir.
/// - Linux: ~/.local/share/notes_app.
Future<String> dataDir() async {
  if (Platform.isAndroid) {
    const ch = MethodChannel('notes/paths');
    final dir = await ch.invokeMethod<String>('filesDir');
    return dir!;
  }
  final forced = Platform.environment['NOTES_DATA_DIR'];
  if (forced != null && forced.isNotEmpty) return forced;
  if (Platform.isWindows) {
    final base = Platform.environment['LOCALAPPDATA'] ?? Directory.systemTemp.path;
    return '$base\\Notes';
  }
  final home = Platform.environment['HOME'] ?? Directory.systemTemp.path;
  return '$home/.local/share/notes_app';
}

String vaultRootIn(String dataDir) => '$dataDir${Platform.pathSeparator}vault';

/// One line per event (start, ready, errors). Printed (logcat tag `flutter`
/// on Android) and, on desktop, appended to `<data dir>/notes.log`.
class AppLog {
  static File? _file;

  static void init(String dir) {
    if (Platform.isAndroid) return;
    try {
      Directory(dir).createSync(recursive: true);
      _file = File('$dir${Platform.pathSeparator}notes.log');
    } catch (_) {}
  }

  static void info(String msg) {
    final line = 'NOTES $msg';
    debugPrint(line);
    LogBuffer.instance.addLine(line);
    try {
      _file?.writeAsStringSync('${clockNow().toIso8601String()} $line\n', mode: FileMode.append);
    } catch (_) {}
  }
}

/// Boot step timings: one `boot: <step> <ms>ms` line through [AppLog], where
/// the number is the time since the Dart main entry. Does nothing until
/// [start] is called.
abstract final class BootTiming {
  static int? _startUs;

  static void start(int startedAtUs) => _startUs = startedAtUs;

  static void mark(String step) {
    final s = _startUs;
    if (s == null) return;
    final ms = (clockNow().microsecondsSinceEpoch - s) / 1000;
    AppLog.info('boot: $step ${ms.toStringAsFixed(0)}ms');
  }
}
