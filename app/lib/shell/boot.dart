// Starting the app: open (or create) the vault, build the stores, show home.
//
// - A new vault (its folder missing or empty) gets a "Getting started" page.
//   With sync configured, those starter pages wait for the first successful
//   pull and are created only if the vault still has no pages then.
// - Built with `--dart-define=SEED_BENCH=true`, a new vault is instead the
//   bench vault: 10,000 generated pages, a 2,000-block "Big page" and the
//   showcase page (for measure.ps1).
// - Built with `--dart-define=SEED_TASKS=<rows>`, a new vault also gets the
//   "Tasks" database with that many rows (seed_tasks).
// - The core's writer is flushed whenever the app may be killed.
// - On Windows, the update checker starts (features/updates).

import 'dart:async';
import 'dart:io';
import 'dart:ui' show AppExitResponse;

import 'package:flutter/widgets.dart';

import '../core/link_previews.dart';
import '../core/models.dart';
import '../core/paths.dart';
import '../core/vault.dart';
import '../features/bakeoff.dart';
import '../features/settings/settings_store.dart';
import '../features/sync/sync_control.dart';
import '../features/updates/update_controller.dart';
import '../features/getting_started.dart';
import '../features/showcase.dart';
import '../features/demo.dart';
import '../state/app_state.dart';
import '../state/ui_state.dart';
import '../ui/ui.dart';
import 'app_shell.dart';
import 'addons.dart';
import 'home.dart';

const seedBench = bool.fromEnvironment('SEED_BENCH');
const seedDemo = bool.fromEnvironment('SEED_DEMO');
const seedTaskRows = int.fromEnvironment('SEED_TASKS');
const benchPages = 10000;
const benchBigPageBlocks = 2000;

class NotesApp extends StatefulWidget {
  const NotesApp({super.key, required this.dataDir, this.startedAtUs});
  final String dataDir;
  final int? startedAtUs;

  @override
  State<NotesApp> createState() => _NotesAppState();
}

class _NotesAppState extends State<NotesApp> {
  late final AppLifecycleListener _lifecycle;
  late final AppSettings _settings = AppSettings.load(File('${widget.dataDir}${Platform.pathSeparator}settings.json'));
  AppState? _app;
  bool _seeding = false;
  int _seeded = 0;
  String? _error;

  @override
  void initState() {
    super.initState();
    registerBuiltins();
    if (Platform.isWindows) {
      UpdateController.current = UpdateController(
        currentTag: effectiveReleaseTag(),
        updatesDir: '${widget.dataDir}${Platform.pathSeparator}updates',
        settings: _settings,
      )..start();
    }
    _lifecycle = AppLifecycleListener(
      onPause: _flush,
      onHide: _flush,
      onExitRequested: () async {
        await _flush();
        return AppExitResponse.exit;
      },
    );
    _boot();
  }

  Future<void> _flush() async {
    final app = _app;
    if (app == null) return;
    try {
      await app.pageStore.lastSave;
      await app.vault.flush();
    } catch (_) {}
  }

  static bool _isNew(String root) {
    final d = Directory(root);
    return !d.existsSync() || d.listSync().isEmpty;
  }

  Future<void> _boot() async {
    final root = _settings.vaultPath ?? vaultRootIn(widget.dataDir);
    try {
      final fresh = _isNew(root);
      // Sync configured: the hub's pages may be on their way, so starter pages
      // wait for the first pull. Not gated by [fresh]: Vault.open creates
      // `.vault/`, so a launch that failed its pull would look non-fresh next time.
      final syncing = _settings.clientUrl != null && _settings.clientToken != null;
      if (fresh && seedBench && !seedDemo) await _seedBench(root);
      if (fresh && seedTaskRows > 0 && !seedDemo) await NotesVault.seedTasks(root: root, rows: seedTaskRows);
      final vault = await Vault.open(root, dataDir: widget.dataDir);
      if (_settings.scheduleRunsHere && (Platform.isWindows || Platform.isLinux)) vault.localRunnerSetAvailable(true).ignore();
      if (fresh) {
        if (seedDemo) {
          await createDemo(vault);
        } else if (seedBench) {
          await createShowcase(vault);
        } else {
          await seedStarterPagesAtBoot(vault, syncing: syncing, createTasks: seedTaskRows == 0);
        }
        await vault.flush();
      }
      await vault.run((v) => v.ensureSpaceColors(), writes: true);
      final ui = UiState.load(File('${widget.dataDir}${Platform.pathSeparator}ui_state.json'), root);
      final links = LinkPreviews(file: File('${widget.dataDir}${Platform.pathSeparator}link_previews.json'));
      final app = AppState(vault, ui: ui, links: links, settings: _settings);
      await app.start();
      AppLog.info('ready pages=${app.pages.count} fresh=$fresh');
      if (!mounted) {
        await app.dispose();
        return;
      }
      setState(() => _app = app);
      unawaited(
        startSyncAtBoot(app.sync, _settings).whenComplete(
          () => app.runNotices.start().catchError((Object e) {
            AppLog.info('run notices failed to start: $e');
          }),
        ),
      );
      if (syncing && !seedDemo && !seedBench) {
        unawaited(seedStarterPagesAfterPull(vault, app.sync.firstSyncOk(), createTasks: seedTaskRows == 0).catchError((Object e) {
          AppLog.info('starter pages after pull failed: $e');
          return false;
        }));
      }
    } catch (e) {
      AppLog.info('open failed: $e');
      if (mounted) setState(() => _error = '$e');
    }
  }

  Future<void> _seedBench(String root) async {
    setState(() => _seeding = true);
    final sw = Stopwatch()..start();
    final timer = Timer.periodic(const Duration(milliseconds: 250), (_) {
      final p = NotesVault.seedProgress(root: root);
      if (mounted) setState(() => _seeded = p);
    });
    try {
      await NotesVault.seed(root: root, pages: benchPages, bigPageBlocks: benchBigPageBlocks);
    } finally {
      timer.cancel();
    }
    Bakeoff.instance.log('seeded pages=$benchPages ms=${sw.elapsedMilliseconds}');
  }

  @override
  void dispose() {
    _lifecycle.dispose();
    UpdateController.current?.dispose();
    UpdateController.current = null;
    _app?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    // NOTES_THEME=light|dark forces a theme on desktop; otherwise the theme
    // setting applies (default: the system's).
    final forced = Platform.environment['NOTES_THEME'];
    final app = _app;
    return ListenableBuilder(
      listenable: _settings,
      builder: (context, _) => NotesShell(
        forceDark: forced == null ? _settings.theme.forceDark : forced == 'dark',
        wrap: (child) => SettingsScope(
          settings: _settings,
          child: app == null ? child : AppScope(state: app, child: child),
        ),
        home: Builder(builder: (context) {
          if (app != null) return HomeScreen(startedAtUs: widget.startedAtUs, restoreLastPage: true);
          return _Boot(
            seeding: _seeding,
            seeded: _seeded,
            error: _error,
            // A vault folder chosen in settings that fails to open.
            onUseDefault: _error == null || _settings.vaultPath == null
                ? null
                : () {
                    _settings.update((s) => s.vaultPath = null);
                    setState(() => _error = null);
                    _boot();
                  },
          );
        }),
      ),
    );
  }
}

class _Boot extends StatelessWidget {
  const _Boot({required this.seeding, required this.seeded, this.error, this.onUseDefault});
  final bool seeding;
  final int seeded;
  final String? error;
  final VoidCallback? onUseDefault;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    if (error != null) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(mainAxisSize: MainAxisSize.min, children: [
            Text('Could not open the vault:\n$error', style: ts(ty.ui, c.textPrimary)),
            if (onUseDefault != null) ...[
              const SizedBox(height: 12),
              TextBtn('Open the default vault instead', onTap: onUseDefault),
            ],
          ]),
        ),
      );
    }
    // Loading: the theme's desk colour, the same one Home sits on.
    if (!seeding) return ColoredBox(color: c.desk, child: const SizedBox.expand());
    const total = benchPages + 1;
    final frac = (seeded / total).clamp(0.0, 1.0);
    return Center(
      child: SizedBox(
        width: 280,
        child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text('Creating the bench vault', style: ts(ty.h3, c.textPrimary)),
          const SizedBox(height: 8),
          Text(frac < 1 ? '$seeded of $total pages' : 'Building the index…',
              style: ts(ty.ui, c.textSecondary, weight: 400)),
          const SizedBox(height: 16),
          ClipRRect(
            borderRadius: BorderRadius.circular(2),
            child: SizedBox(
              height: 4,
              child: Stack(children: [
                Container(color: c.bgTertiary),
                FractionallySizedBox(widthFactor: frac, child: Container(color: c.accent)),
              ]),
            ),
          ),
        ]),
      ),
    );
  }
}
