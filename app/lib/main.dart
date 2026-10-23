// Scratch: a notes app over the Rust vault core, through
// flutter_rust_bridge. WidgetsApp only (no Material or Cupertino); every
// screen is built from the UI kit in lib/ui. Layers: wiki/architecture/app-state.md.

import 'package:flutter/widgets.dart';

import 'core/clock.dart';
import 'core/paths.dart';
import 'features/bakeoff.dart';
import 'feedback/log_buffer.dart';
import 'shell/boot.dart';
import 'src/rust/frb_generated.dart';

Future<void> main() async {
  final startedAtUs = clockNow().microsecondsSinceEpoch;
  BootTiming.start(startedAtUs);
  WidgetsFlutterBinding.ensureInitialized();
  final dir = await dataDir();
  AppLog.init(dir);
  BootTiming.mark('dart main');
  BootTiming.mark('before RustLib.init');
  await RustLib.init();
  BootTiming.mark('after RustLib.init');
  installErrorHooks();
  AppLog.info('start');
  Bakeoff.instance.init(logDir: dir);
  runApp(NotesApp(dataDir: dir, startedAtUs: startedAtUs));
  WidgetsBinding.instance.addPostFrameCallback((_) => BootTiming.mark('first frame'));
}
