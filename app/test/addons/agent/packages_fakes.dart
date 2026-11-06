// A fake bridge for the runtime packages UI: the test decides the packages'
// state and drives each install by hand through its stream controller.

import 'dart:async';

import 'package:notes_app/addons/agent/packages_store.dart';

const megabyte = 1024 * 1024;

PackageInfo claudeNotInstalled({int mb = 168}) => PackageInfo(
  name: claudeCodePackage,
  version: '2.0.0',
  downloadBytes: mb * megabyte,
  state: PackageState.notInstalled,
);

PackageInfo claudeInstalled({int mb = 168, int onDiskMb = 412}) => PackageInfo(
  name: claudeCodePackage,
  version: '2.0.0',
  downloadBytes: mb * megabyte,
  state: PackageState.installed,
  installedVersion: '2.0.0',
  installedBytes: onDiskMb * megabyte,
);

PackageInfo claudeUpdate({int mb = 168}) => PackageInfo(
  name: claudeCodePackage,
  version: '2.1.0',
  downloadBytes: mb * megabyte,
  state: PackageState.updateAvailable,
  installedVersion: '2.0.0',
  installedBytes: 412 * megabyte,
);

PackageProgress downloading(int done, int total) => PackageProgress(
  phase: PackagePhase.downloading,
  done: done,
  total: total,
);

PackageProgress step(PackagePhase phase) =>
    PackageProgress(phase: phase, done: 0, total: 0);

class FakePackagesBackend implements PackagesBackend {
  FakePackagesBackend(List<PackageInfo> packages) {
    for (final p in packages) {
      state[p.name] = p;
    }
  }

  final state = <String, PackageInfo>{};

  /// A thrown error from `list`, when set.
  Object? listFails;

  /// A thrown error from `install` itself (before any stream), when set.
  Object? installThrows;

  /// Runs when `remove` is called, before the state changes.
  void Function()? onRemove;

  /// A thrown error from `remove` after the package is gone (files left).
  Object? removeThrows;

  final installCalls = <String>[];
  final removeCalls = <String>[];

  /// One controller per `install` call, in order.
  final controllers = <StreamController<PackageProgress>>[];

  StreamController<PackageProgress> get lastInstall => controllers.last;

  @override
  Future<List<PackageInfo>> list() async {
    final e = listFails;
    if (e != null) throw e;
    return [...state.values];
  }

  @override
  Future<PackageInfo> info(String name) async =>
      state[name] ??
      PackageInfo(
        name: name,
        version: '',
        downloadBytes: 0,
        state: PackageState.notAvailable,
      );

  @override
  Stream<PackageProgress> install(String name) {
    installCalls.add(name);
    final e = installThrows;
    if (e != null) throw e;
    final c = StreamController<PackageProgress>();
    controllers.add(c);
    return c.stream;
  }

  @override
  Future<void> remove(String name) async {
    removeCalls.add(name);
    onRemove?.call();
    state[name] = PackageInfo(
      name: name,
      version: state[name]?.version ?? '',
      downloadBytes: state[name]?.downloadBytes ?? 0,
      state: PackageState.notInstalled,
    );
    final e = removeThrows;
    if (e != null) throw e;
  }
}
