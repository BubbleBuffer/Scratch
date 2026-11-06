// PackagesStore over a fake backend: the state of each package, one install
// per package, progress, failure and retry, removal, and the card's rules.
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show AnyhowException;
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/agent_runtime_card.dart';
import 'package:notes_app/addons/agent/packages_store.dart';

import 'packages_fakes.dart';

const _name = claudeCodePackage;

void main() {
  test('sizes read like 168 MB', () {
    expect(packageSize(168 * megabyte), '168 MB');
    expect(packageSize(168 * megabyte + 300 * 1024), '168 MB');
    expect(packageSize(512 * 1024), '512 KB');
    expect(packageSize(12), '12 B');
    expect(packageSize(3 * 1024 * megabyte ~/ 2), '1.5 GB');
    expect(packageTitle(_name), 'Claude Code');
    expect(packageTitle('other'), 'other');
  });

  test('refresh reads the list; a failed read leaves it empty', () async {
    final backend = FakePackagesBackend([claudeNotInstalled()]);
    final store = PackagesStore(backend: backend);
    expect(store.loaded, isFalse);
    await store.ensureLoaded();
    expect(store.loaded, isTrue);
    expect(store.infoOf(_name)?.state, PackageState.notInstalled);
    expect(store.statusOf(_name), isA<PackageIdle>());

    backend.listFails = 'boom';
    await store.refresh();
    expect(store.packages, isEmpty);
    expect(store.loaded, isTrue);
    store.dispose();
  });

  test('install goes idle, installing with progress, installed', () async {
    final backend = FakePackagesBackend([claudeNotInstalled()]);
    final store = PackagesStore(backend: backend);
    await store.refresh();
    final seen = <PackageStatus>[];
    store.addListener(() => seen.add(store.statusOf(_name)));
    final finished = <String>[];
    store.installs.listen(finished.add);

    final done = store.install(_name);
    expect(store.statusOf(_name), isA<PackageInstalling>());
    expect(store.installing, isTrue);
    expect(backend.installCalls, [_name]);

    backend.lastInstall.add(downloading(42, 100));
    await pumpEventQueue();
    final mid = store.statusOf(_name) as PackageInstalling;
    expect(mid.phase, PackagePhase.downloading);
    expect(mid.percent, 42);

    backend.lastInstall.add(step(PackagePhase.verifying));
    backend.lastInstall.add(step(PackagePhase.extracting));
    backend.lastInstall.add(step(PackagePhase.installing));
    await pumpEventQueue();
    expect(
      (store.statusOf(_name) as PackageInstalling).phase,
      PackagePhase.installing,
    );
    expect(finished, isEmpty, reason: 'not before the install ends');

    backend.state[_name] = claudeInstalled();
    await backend.lastInstall.close();
    expect(await done, isTrue);
    expect(store.statusOf(_name), isA<PackageIdle>());
    expect(store.infoOf(_name)?.state, PackageState.installed);
    expect(store.justInstalled(_name), isTrue);
    expect(store.installing, isFalse);
    expect(finished, [_name]);
    expect(
      seen.whereType<PackageInstalling>().map((s) => s.phase).toSet(),
      containsAll(PackagePhase.values),
    );
    store.dispose();
  });

  test('a stream error ends as failed; Retry installs again', () async {
    final backend = FakePackagesBackend([claudeNotInstalled()]);
    final store = PackagesStore(backend: backend);
    await store.refresh();
    final finished = <String>[];
    store.installs.listen(finished.add);

    final first = store.install(_name);
    backend.lastInstall.addError('download failed the checksum check');
    expect(await first, isFalse);
    final status = store.statusOf(_name);
    expect(status, isA<PackageFailed>());
    expect(
      (status as PackageFailed).message,
      'download failed the checksum check',
    );
    expect(store.installing, isFalse);
    expect(store.justInstalled(_name), isFalse);
    expect(finished, isEmpty, reason: 'a failed install announces nothing');
    expect(store.infoOf(_name)?.state, PackageState.notInstalled);

    final retry = store.install(_name);
    expect(store.statusOf(_name), isA<PackageInstalling>());
    expect(backend.installCalls, [_name, _name]);
    backend.state[_name] = claudeInstalled();
    await backend.lastInstall.close();
    expect(await retry, isTrue);
    expect(store.statusOf(_name), isA<PackageIdle>());
    expect(finished, [_name]);
    store.dispose();
  });

  test('a bridge error shows its message without the wrapper', () async {
    final backend = FakePackagesBackend([claudeNotInstalled()]);
    final store = PackagesStore(backend: backend);
    await store.refresh();
    final install = store.install(_name);
    backend.lastInstall.addError(
      AnyhowException('download failed, check the connection: refused'),
    );
    expect(await install, isFalse);
    expect(
      (store.statusOf(_name) as PackageFailed).message,
      'download failed, check the connection: refused',
    );
    store.dispose();
  });

  test('remove stops the runtime first and starts it after the list', () async {
    final backend = FakePackagesBackend([claudeInstalled()]);
    final store = PackagesStore(backend: backend);
    await store.refresh();
    final steps = <String>[];
    backend.onRemove = () => steps.add('delete');
    store.beforeRemove = (_) async => steps.add('stop');
    store.afterRemove = (_) async {
      steps.add('start:${store.infoOf(_name)?.state.name}');
    };
    await store.remove(_name);
    expect(steps, ['stop', 'delete', 'start:notInstalled']);
    expect(store.isRemoving(_name), isFalse);
    store.dispose();
  });

  test('remove does nothing while the blocker gives a reason', () async {
    final backend = FakePackagesBackend([claudeInstalled()]);
    final store = PackagesStore(backend: backend);
    await store.refresh();
    var hooks = 0;
    store.beforeRemove = (_) async => hooks++;
    store.afterRemove = (_) async => hooks++;
    store.removeBlocker = (_) => 'a reply is running';
    expect(store.removeBlockedReason(_name), 'a reply is running');
    await store.remove(_name);
    expect(backend.removeCalls, isEmpty);
    expect(hooks, 0);
    expect(store.infoOf(_name)?.state, PackageState.installed);
    store.removeBlocker = null;
    await store.remove(_name);
    expect(backend.removeCalls, [_name]);
    store.dispose();
  });

  test('files left behind: removed, message shown, runtime restarts', () async {
    final backend = FakePackagesBackend([claudeInstalled()])
      ..removeThrows = AnyhowException(
        'Removed. Some files are still in use and will be deleted the next '
        'time Scratch starts.',
      );
    final store = PackagesStore(backend: backend);
    await store.refresh();
    var restarted = false;
    store.afterRemove = (_) async => restarted = true;
    await store.remove(_name);
    expect(store.infoOf(_name)?.state, PackageState.notInstalled);
    expect(
      (store.statusOf(_name) as PackageFailed).message,
      startsWith('Removed. Some files are still in use'),
    );
    expect(restarted, isTrue);
    store.dispose();
  });

  test('an error thrown by the call ends as failed too', () async {
    final backend = FakePackagesBackend([claudeNotInstalled()])
      ..installThrows = Exception('not enough disk space');
    final store = PackagesStore(backend: backend);
    await store.refresh();
    expect(await store.install(_name), isFalse);
    expect(
      (store.statusOf(_name) as PackageFailed).message,
      'not enough disk space',
    );
    store.dismissFailure(_name);
    expect(store.statusOf(_name), isA<PackageIdle>());
    store.dispose();
  });

  test('one install per package: a second call joins the first', () async {
    final backend = FakePackagesBackend([claudeNotInstalled()]);
    final store = PackagesStore(backend: backend);
    await store.refresh();
    final a = store.install(_name);
    final b = store.install(_name);
    expect(identical(a, b), isTrue);
    expect(backend.installCalls, [_name]);
    backend.state[_name] = claudeInstalled();
    await backend.lastInstall.close();
    expect(await a, isTrue);
    expect(await b, isTrue);
    expect(backend.installCalls, hasLength(1));
    store.dispose();
  });

  test('remove deletes the package; it waits while an install runs', () async {
    final backend = FakePackagesBackend([claudeInstalled()]);
    final store = PackagesStore(backend: backend);
    await store.refresh();
    expect(store.infoOf(_name)?.state, PackageState.installed);
    await store.remove(_name);
    expect(backend.removeCalls, [_name]);
    expect(store.infoOf(_name)?.state, PackageState.notInstalled);

    final running = store.install(_name);
    await store.remove(_name);
    expect(backend.removeCalls, hasLength(1), reason: 'waits for the install');
    backend.lastInstall.addError('download failed, check the connection: x');
    await running;
    store.dispose();
  });

  group('card rules', () {
    RuntimeCardMode mode({
      bool selected = true,
      bool? available = false,
      PackageInfo? info,
      PackageStatus status = const PackageIdle(),
      bool ready = false,
    }) => runtimeCardMode(
      selected: selected,
      available: available,
      info: info,
      status: status,
      ready: ready,
    );

    test('shows for a missing package when Claude Code is the model', () {
      expect(mode(info: claudeNotInstalled()), RuntimeCardMode.block);
      expect(mode(info: claudeUpdate()), RuntimeCardMode.block);
    });

    test('hidden when the model is another provider or nothing is known', () {
      expect(
        mode(selected: false, info: claudeNotInstalled()),
        RuntimeCardMode.hidden,
      );
      expect(mode(info: null), RuntimeCardMode.hidden);
      expect(
        mode(available: null, info: claudeNotInstalled()),
        RuntimeCardMode.hidden,
      );
      final none = PackageInfo(
        name: _name,
        version: '',
        downloadBytes: 0,
        state: PackageState.notAvailable,
      );
      expect(mode(info: none), RuntimeCardMode.hidden);
    });

    test('an update is compact while Claude Code works; none when current', () {
      expect(
        mode(available: true, info: claudeUpdate()),
        RuntimeCardMode.compact,
      );
      expect(
        mode(available: true, info: claudeInstalled()),
        RuntimeCardMode.hidden,
      );
      expect(
        mode(available: true, info: claudeNotInstalled()),
        RuntimeCardMode.hidden,
      );
    });

    test('an install in flight and Ready keep the card', () {
      const working = PackageInstalling(PackagePhase.verifying);
      expect(
        mode(available: null, info: claudeNotInstalled(), status: working),
        RuntimeCardMode.block,
      );
      expect(
        mode(available: true, info: claudeUpdate(), status: working),
        RuntimeCardMode.compact,
      );
      expect(
        mode(available: null, info: claudeInstalled(), ready: true),
        RuntimeCardMode.block,
      );
      expect(
        mode(available: true, info: claudeInstalled(), ready: true),
        RuntimeCardMode.hidden,
      );
    });
  });
}
