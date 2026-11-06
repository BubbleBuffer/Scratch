// The package API on the real host bridge. CI builds the Linux bridge with
// the checked-in manifest, which lists no package, so the platform is
// linux-x64 and a fresh data folder has nothing installed.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/packages_store.dart';
import 'package:notes_app/src/rust/api/packages.dart';

import 'harness.dart';

void main() {
  setUpAll(initCore);

  late Directory dir;
  setUp(() => dir = Directory.systemTemp.createTempSync('packages_bridge'));
  tearDown(() => dir.deleteSync(recursive: true));

  test(
    'the platform is linux-x64',
    () => expect(packagesPlatform(), 'linux-x64'),
    skip: Platform.isLinux ? false : 'the Linux bridge only',
  );

  test(
    'a fresh data folder lists no package',
    () {
      expect(packagesList(dataDir: dir.path), isEmpty);
      expect(
        packageInfo(dataDir: dir.path, name: 'claude-code').state,
        PackageState.notAvailable,
      );
    },
    skip: Platform.isLinux ? false : 'the Linux bridge only',
  );

  test(
    'the store over the real bridge reads an empty list and hides itself',
    () async {
      final store = PackagesStore(
        backend: _DirBackend(dir.path),
      );
      await store.refresh();
      expect(store.loaded, isTrue);
      expect(store.packages, isEmpty);
      store.dispose();
    },
    skip: Platform.isLinux ? false : 'the Linux bridge only',
  );

  // The bridge used to hand a failed install back as the call's return value,
  // which a stream function drops: the stream just ended, the store took it
  // for a finished install and the card stuck on "Ready".
  test(
    'a failed install over the real bridge ends as failed with its message',
    () async {
      final store = PackagesStore(backend: _DirBackend(dir.path));
      final finished = <String>[];
      store.installs.listen(finished.add);
      // The checked-in manifest lists no package: the bridge refuses.
      expect(await store.install('claude-code'), isFalse);
      final status = store.statusOf('claude-code');
      expect(status, isA<PackageFailed>());
      expect((status as PackageFailed).message, contains('not available'));
      expect(store.justInstalled('claude-code'), isFalse);
      expect(finished, isEmpty);
      store.dispose();
    },
    skip: Platform.isLinux ? false : 'the Linux bridge only',
  );
}

/// The bridge backend on a temp folder instead of the app's data folder.
class _DirBackend implements PackagesBackend {
  _DirBackend(this.dir);
  final String dir;

  @override
  Future<List<PackageInfo>> list() async => packagesList(dataDir: dir);

  @override
  Future<PackageInfo> info(String name) async =>
      packageInfo(dataDir: dir, name: name);

  @override
  Stream<PackageProgress> install(String name) =>
      packageInstall(dataDir: dir, name: name);

  @override
  Future<void> remove(String name) => packageRemove(dataDir: dir, name: name);
}
