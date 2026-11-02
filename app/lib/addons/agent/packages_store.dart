// Runtime packages (wiki/addons/packages.md) as the UI sees them: each
// package's state, the install in flight and its progress, shared by the
// agent pane's download card and Settings > Agent > Installed packages.
//
// - The bridge is behind [PackagesBackend], so tests drive the store with a
//   fake; [BridgePackagesBackend] is the real one (data folder from
//   `dataDir()`).
// - One install per package: a second `install` while one runs joins it.
// - Status per package: idle, installing (phase and bytes) or failed (a
//   message that names the state). Stream errors and thrown errors both end
//   as failed. A finished install announces itself on [installs]; a failed
//   one does not.
// - No state is kept on disk: the install folder is the truth, read again
//   after every install and removal.

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/paths.dart';
import '../../src/rust/api/packages.dart' as bridge;
import '../../src/rust/api/packages.dart'
    show PackageInfo, PackagePhase, PackageProgress, PackageState;

export '../../src/rust/api/packages.dart'
    show PackageInfo, PackagePhase, PackageProgress, PackageState;

/// The bridge's package API, as the store uses it.
abstract class PackagesBackend {
  Future<List<PackageInfo>> list();
  Future<PackageInfo> info(String name);

  /// Progress while it installs; ends when installed, errors when it fails.
  Stream<PackageProgress> install(String name);
  Future<void> remove(String name);
}

class BridgePackagesBackend implements PackagesBackend {
  const BridgePackagesBackend();

  @override
  Future<List<PackageInfo>> list() async =>
      bridge.packagesList(dataDir: await dataDir());

  @override
  Future<PackageInfo> info(String name) async =>
      bridge.packageInfo(dataDir: await dataDir(), name: name);

  @override
  Stream<PackageProgress> install(String name) async* {
    yield* bridge.packageInstall(dataDir: await dataDir(), name: name);
  }

  @override
  Future<void> remove(String name) async =>
      bridge.packageRemove(dataDir: await dataDir(), name: name);
}

/// What one package is doing now.
sealed class PackageStatus {
  const PackageStatus();
}

class PackageIdle extends PackageStatus {
  const PackageIdle();
}

class PackageInstalling extends PackageStatus {
  const PackageInstalling(this.phase, {this.done = 0, this.total = 0});
  final PackagePhase phase;

  /// Bytes; they only mean something while [phase] is downloading.
  final int done;
  final int total;

  /// Whole percent of the download (0 until the size is known).
  int get percent =>
      total <= 0 ? 0 : (done * 100 ~/ total).clamp(0, 100).toInt();

  @override
  bool operator ==(Object other) =>
      other is PackageInstalling &&
      other.phase == phase &&
      other.done == done &&
      other.total == total;

  @override
  int get hashCode => Object.hash(phase, done, total);
}

class PackageFailed extends PackageStatus {
  const PackageFailed(this.message);
  final String message;

  @override
  bool operator ==(Object other) =>
      other is PackageFailed && other.message == message;

  @override
  int get hashCode => message.hashCode;
}

/// The package that carries the Claude Code provider.
const claudeCodePackage = 'claude-code';

/// The name shown for [package].
String packageTitle(String package) =>
    package == claudeCodePackage ? 'Claude Code' : package;

/// A download size as "168 MB" (KB below a megabyte, GB from a gigabyte).
String packageSize(int bytes) {
  const kb = 1024;
  const mb = 1024 * 1024;
  const gb = 1024 * 1024 * 1024;
  if (bytes >= gb) return '${(bytes / gb).toStringAsFixed(1)} GB';
  if (bytes >= mb) return '${(bytes / mb).round()} MB';
  if (bytes >= kb) return '${(bytes / kb).round()} KB';
  return '$bytes B';
}

class PackagesStore extends ChangeNotifier {
  PackagesStore({PackagesBackend? backend})
    : _backend = backend ?? const BridgePackagesBackend();

  final PackagesBackend _backend;

  static PackagesStore? _shared;

  /// The one store the app uses (the card and Settings share its progress).
  static PackagesStore get shared => _shared ??= PackagesStore();

  /// Tests: use [store] as the shared one (null: a fresh real one next time).
  @visibleForTesting
  static set shared(PackagesStore? store) => _shared = store;

  /// Every package this build lists for this platform, plus other installed
  /// ones, as last read. Empty where the build lists none.
  List<PackageInfo> get packages => _packages;
  List<PackageInfo> _packages = const [];

  /// Whether [packages] was read at least once.
  bool get loaded => _loaded;
  bool _loaded = false;

  final _status = <String, PackageStatus>{};
  final _running = <String, Future<bool>>{};
  final _ready = <String>{};
  final _removing = <String>{};
  final _installed = StreamController<String>.broadcast(sync: true);
  Future<void>? _loading;
  bool _disposed = false;

  /// Names of packages as their install finishes (not when one fails).
  Stream<String> get installs => _installed.stream;

  PackageInfo? infoOf(String name) {
    for (final p in _packages) {
      if (p.name == name) return p;
    }
    return null;
  }

  PackageStatus statusOf(String name) => _status[name] ?? const PackageIdle();

  /// Whether an install is running (any package): removal waits for it.
  bool get installing => _running.isNotEmpty;

  bool isRemoving(String name) => _removing.contains(name);

  /// Whether [name] was installed during this run and nothing has touched it
  /// since (the card says "Ready" while the agent looks for it).
  bool justInstalled(String name) => _ready.contains(name);

  /// Reads the list once; later calls share that read.
  Future<void> ensureLoaded() => _loading ??= refresh();

  /// Reads every package's state again. Never throws: a failed read leaves
  /// the list empty (the UI then shows nothing).
  Future<void> refresh() async {
    List<PackageInfo> list;
    try {
      list = await _backend.list();
    } catch (e) {
      AppLog.info('packages list: $e');
      list = const [];
    }
    _packages = list;
    _loaded = true;
    _notify();
  }

  Future<void> _refreshOne(String name) async {
    try {
      final info = await _backend.info(name);
      _packages = [
        for (final p in _packages)
          if (p.name != name) p,
        if (info.state != PackageState.notAvailable) info,
      ]..sort((a, b) => a.name.compareTo(b.name));
    } catch (e) {
      AppLog.info('package $name: $e');
      return refresh();
    }
  }

  /// Installs (or updates) [name]. One run per package: while it runs, this
  /// returns the same future. True when it installed; false when it failed
  /// (the status then says why).
  Future<bool> install(String name) {
    final running = _running[name];
    if (running != null) return running;
    // Registered before the run starts, so its first notification already
    // reads [installing] as true.
    final done = Completer<bool>();
    _running[name] = done.future;
    _install(name).then<void>(done.complete).whenComplete(() {
      _running.remove(name);
      _notify();
    });
    return done.future;
  }

  Future<bool> _install(String name) async {
    _ready.remove(name);
    _status[name] = const PackageInstalling(PackagePhase.downloading);
    _notify();
    try {
      await for (final p in _backend.install(name)) {
        _status[name] = PackageInstalling(
          p.phase,
          done: p.done.toInt(),
          total: p.total.toInt(),
        );
        _notify();
      }
    } catch (e) {
      _status[name] = PackageFailed(_message(e));
      _notify();
      return false;
    }
    _status[name] = const PackageIdle();
    _ready.add(name);
    await _refreshOne(name);
    _notify();
    if (!_disposed) _installed.add(name);
    return true;
  }

  /// Removes every installed version of [name]. Does nothing while any
  /// install runs.
  Future<void> remove(String name) async {
    if (installing || _removing.contains(name)) return;
    _removing.add(name);
    _ready.remove(name);
    _notify();
    try {
      await _backend.remove(name);
      _status.remove(name);
    } catch (e) {
      _status[name] = PackageFailed(_message(e));
    }
    _removing.remove(name);
    await _refreshOne(name);
    _notify();
  }

  /// Forgets a failure (the card's error goes away).
  void dismissFailure(String name) {
    if (_status[name] is! PackageFailed) return;
    _status.remove(name);
    _notify();
  }

  /// The error's own text: the bridge throws the message it names the state
  /// in, not a stack of wrappers.
  static String _message(Object e) {
    var text = '$e'.trim();
    if (text.startsWith('Exception: ')) text = text.substring(11).trim();
    return text.isEmpty ? 'download failed' : text;
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    unawaited(_installed.close());
    super.dispose();
  }
}

