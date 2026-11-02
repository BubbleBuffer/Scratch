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
import 'dart:convert';
import 'dart:io';
import 'dart:ui' show AppExitResponse;

import 'package:flutter/widgets.dart';
import 'package:package_info_plus/package_info_plus.dart';

import '../core/link_previews.dart';
import '../core/models.dart';
import '../core/paths.dart';
import '../core/vault.dart';
import '../features/bakeoff.dart';
import '../features/settings/settings_screen.dart' show appVersion;
import '../features/settings/settings_store.dart';
import '../features/sync/sync_control.dart';
import '../features/updates/update_controller.dart';
import '../features/getting_started.dart';
import '../features/showcase.dart';
import '../remote/app_remote.dart';
import '../features/demo.dart';
import '../state/app_state.dart';
import '../state/ui_state.dart';
import '../ui/ui.dart';
import 'app_shell.dart';
import 'addons.dart';
import 'home.dart';

// Boot timing runs also take SCRATCH_SEED_BENCH=1 and SCRATCH_SEED_PAGES=<n>
// (environment, desktop) so one build can measure small and large vaults.
final seedBench = const bool.fromEnvironment('SEED_BENCH') || Platform.environment['SCRATCH_SEED_BENCH'] == '1';
const seedDemo = bool.fromEnvironment('SEED_DEMO');
const seedTaskRows = int.fromEnvironment('SEED_TASKS');
final benchPages = int.tryParse(Platform.environment['SCRATCH_SEED_PAGES'] ?? '') ?? 10000;
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
  AppRemote? _remote;
  Timer? _frameFallback;
  bool _seeding = false;
  // A rebuild of the search index during open: pages done of total.
  bool _updatingIndex = false;
  int _indexDone = 0;
  int _indexTotal = 0;
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

  void _readIndexProgress(String root, DateTime openedAt) {
    try {
      final file = File('$root${Platform.pathSeparator}.vault${Platform.pathSeparator}index-update.json');
      // A file from an earlier run (one that died) is older than this open.
      if (!file.existsSync() || file.lastModifiedSync().isBefore(openedAt)) return;
      final j = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
      if (!mounted) return;
      setState(() {
        _updatingIndex = true;
        _indexDone = (j['done'] as num).toInt();
        _indexTotal = (j['total'] as num).toInt();
      });
    } catch (_) {
      // Half written or gone: the next tick reads it again.
    }
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
      BootTiming.mark('before Vault.open');
      // A breaking index change rebuilds the index inside open; the core
      // reports its progress in `.vault/index-update.json` (only when it takes
      // more than a moment), which shows as the "Updating search index" screen.
      final openedAt = DateTime.now().subtract(const Duration(seconds: 1));
      final poll = Timer.periodic(const Duration(milliseconds: 250), (_) => _readIndexProgress(root, openedAt));
      final Vault vault;
      try {
        vault = await Vault.open(root, dataDir: widget.dataDir);
      } finally {
        poll.cancel();
      }
      if (_updatingIndex && mounted) setState(() => _updatingIndex = false);
      BootTiming.mark('after Vault.open');
      if (_settings.scheduleRunsHere && (Platform.isWindows || Platform.isLinux)) {
        vault.localRunnerSetAvailable(true).whenComplete(() => BootTiming.mark('localRunnerSetAvailable done')).ignore();
      }
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
      BootTiming.mark('before ensureSpaceColors');
      await vault.run((v) => v.ensureSpaceColors(), writes: true);
      BootTiming.mark('after ensureSpaceColors');
      final ui = UiState.load(File('${widget.dataDir}${Platform.pathSeparator}ui_state.json'), root);
      final links = LinkPreviews(file: File('${widget.dataDir}${Platform.pathSeparator}link_previews.json'));
      final app = AppState(vault, ui: ui, links: links, settings: _settings);
      BootTiming.mark('before app.start');
      await app.start(startReminders: false);
      BootTiming.mark('after app.start');
      AppLog.info('ready pages=${app.pages.count} fresh=$fresh');
      if (!mounted) {
        await app.dispose();
        return;
      }
      setState(() => _app = app);
      BootTiming.mark('after setState');
      // After Home's first frame: the reminder plan and the agent / AI start.
      final firstFrame = Completer<void>();
      WidgetsBinding.instance.addPostFrameCallback((_) {
        BootTiming.mark('first frame after setState');
        _frameFallback?.cancel();
        if (!firstFrame.isCompleted) firstFrame.complete();
        unawaited(app.reminders.start().catchError((Object e) {
          AppLog.info('reminders failed to start: $e');
        }).whenComplete(() => BootTiming.mark('reminders.start (after first frame)')));
      });
      // A frame is not scheduled when the window is hidden; do not wait forever.
      _frameFallback = Timer(const Duration(seconds: 3), () {
        if (!firstFrame.isCompleted) firstFrame.complete();
      });
      unawaited(_startRemote().whenComplete(() => BootTiming.mark('remote start done')));
      unawaited(
        startSyncAtBoot(app.sync, _settings, afterFirstFrame: firstFrame.future).whenComplete(() {
          BootTiming.mark('startSyncAtBoot done');
          return app.runNotices.start().catchError((Object e) {
            AppLog.info('run notices failed to start: $e');
          }).whenComplete(() => BootTiming.mark('runNotices.start done'));
        }),
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

  /// The app remote-control endpoint (wiki/integrations/app-remote.md): Windows
  /// and Linux desktop only, off with SCRATCH_REMOTE=0. Never blocks or fails
  /// the boot.
  Future<void> _startRemote() async {
    if (!(Platform.isWindows || Platform.isLinux)) return;
    if (Platform.environment['SCRATCH_REMOTE'] == '0' || Platform.environment.containsKey('FLUTTER_TEST') || _remote != null) return;
    try {
      var version = appVersion;
      try {
        version = (await PackageInfo.fromPlatform()).version;
      } catch (_) {}
      final remote = await AppRemote.start(dataDir: widget.dataDir, app: () => _app, version: version);
      if (!mounted) {
        await remote.stop();
        return;
      }
      _remote = remote;
    } catch (e) {
      AppLog.info('remote control failed to start: $e');
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
    _frameFallback?.cancel();
    _lifecycle.dispose();
    unawaited(_remote?.stop());
    _remote = null;
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
            updatingIndex: _updatingIndex,
            indexDone: _indexDone,
            indexTotal: _indexTotal,
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
  const _Boot({
    required this.seeding,
    required this.seeded,
    this.updatingIndex = false,
    this.indexDone = 0,
    this.indexTotal = 0,
    this.error,
    this.onUseDefault,
  });
  final bool seeding;
  final int seeded;
  final bool updatingIndex;
  final int indexDone;
  final int indexTotal;
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
    if (!seeding && !updatingIndex) return ColoredBox(color: c.desk, child: const SizedBox.expand());
    final total = updatingIndex ? indexTotal : benchPages + 1;
    final done = updatingIndex ? indexDone : seeded;
    final frac = total == 0 ? 0.0 : (done / total).clamp(0.0, 1.0);
    return Center(
      child: SizedBox(
        width: 280,
        child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text(updatingIndex ? 'Updating search index' : 'Creating the bench vault', style: ts(ty.h3, c.textPrimary)),
          const SizedBox(height: 8),
          Text(frac < 1 ? '$done of $total pages' : 'Building the index…',
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
