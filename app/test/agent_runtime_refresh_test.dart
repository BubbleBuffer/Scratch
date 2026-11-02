// After a Claude Code package install the agent looks for the runtime again
// (AgentStore.refreshRuntime): a finished install re-opens it once; a failed
// install does not. The package backend is a fake; the agent is the real
// core with the fake extension (see agent_test.dart).
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/packages_store.dart';

import 'addons/agent/packages_fakes.dart';
import 'agent_test.dart' show fakeExtensionSkipReason, openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);

  group('Agent runtime refresh', () {
    appTest('a finished claude-code install re-opens the agent once', (
      tester,
    ) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      final backend = FakePackagesBackend([claudeNotInstalled()]);
      final packages = PackagesStore(backend: backend);
      addTearDown(packages.dispose);
      store.usePackages(packages);
      await packages.refresh();
      expect(store.packages, same(packages));
      expect(store.runtimeRefreshes, 0);

      final install = packages.install(claudeCodePackage);
      await tester.pump();
      backend.state[claudeCodePackage] = claudeInstalled();
      await backend.lastInstall.close();
      expect(await finishAsync(tester, install), isTrue);
      await settleUntil(tester, () => store.runtimeRefreshes == 1);
      await settle(tester);

      expect(store.runtimeRefreshes, 1, reason: 'once, not per notification');
      expect(store.openError, isNull);
      expect(store.hostInfo, isNotNull, reason: 'open again');
    });

    appTest('a failed install does not re-open it', (tester) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      final backend = FakePackagesBackend([claudeNotInstalled()]);
      final packages = PackagesStore(backend: backend);
      addTearDown(packages.dispose);
      store.usePackages(packages);
      await packages.refresh();

      final install = packages.install(claudeCodePackage);
      await tester.pump();
      backend.lastInstall.addError('download failed the checksum check');
      expect(await finishAsync(tester, install), isFalse);
      await settle(tester);

      expect(store.runtimeRefreshes, 0);
      expect(store.hostInfo, isNotNull, reason: 'still the same open agent');
    });

    appTest('another package finishing does not re-open it', (tester) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      final other = PackageInfo(
        name: 'other',
        version: '1.0.0',
        downloadBytes: megabyte,
        state: PackageState.notInstalled,
      );
      final backend = FakePackagesBackend([other]);
      final packages = PackagesStore(backend: backend);
      addTearDown(packages.dispose);
      store.usePackages(packages);
      await packages.refresh();

      final install = packages.install('other');
      await tester.pump();
      await backend.lastInstall.close();
      expect(await finishAsync(tester, install), isTrue);
      await settle(tester);
      expect(store.runtimeRefreshes, 0);
    });
  }, skip: fakeExtensionSkipReason);
}
