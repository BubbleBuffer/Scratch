// After a Claude Code package install the agent looks for the runtime again
// (AgentStore.refreshRuntime): a finished install re-opens it once; a failed
// install does not. The package backend is a fake; the agent is the real
// core with the fake extension (see agent_test.dart).
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/agent_store.dart';
import 'package:notes_app/addons/agent/packages_store.dart';
import 'package:notes_app/src/rust/api/agent.dart' show AgentEvent;

import 'addons/agent/packages_fakes.dart';
import 'agent_test.dart' show fakeExtensionSkipReason, openAgentStore;
import 'harness.dart';

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

    appTest('removing claude-code stops the agent first, then re-opens it', (
      tester,
    ) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      final backend = FakePackagesBackend([claudeInstalled()]);
      final packages = PackagesStore(backend: backend);
      addTearDown(packages.dispose);
      store.usePackages(packages);
      await packages.refresh();
      expect(store.hostInfo, isNotNull);
      expect(store.runtimeRefreshes, 0);

      // The folder is deleted while nothing runs from it.
      Object? hostWhenDeleting = 'not called';
      backend.onRemove = () => hostWhenDeleting = store.hostInfo;
      final removing = packages.remove(claudeCodePackage);
      await finishAsync(tester, removing);
      await settleUntil(tester, () => store.runtimeRefreshes == 1);
      await settle(tester);

      expect(hostWhenDeleting, isNull, reason: 'the runtime was stopped');
      expect(store.runtimeRefreshes, 1);
      expect(store.hostInfo, isNotNull, reason: 'running again');
      expect(packages.infoOf(claudeCodePackage)?.state,
          PackageState.notInstalled);
    });

    appTest('a running reply turns Remove off until it ends', (tester) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      final backend = FakePackagesBackend([claudeInstalled()]);
      final packages = PackagesStore(backend: backend);
      addTearDown(packages.dispose);
      store.usePackages(packages);
      await packages.refresh();
      expect(packages.removeBlockedReason(claudeCodePackage), isNull);

      AgentEvent run(String kind) => AgentEvent(
        kind: kind,
        sequence: 0,
        durable: false,
        sessionId: 'other-chat',
        runId: 'run-1',
        createdAt: '2026-11-06T00:00:00Z',
        isError: false,
        skipped: 0,
        json: '{}',
      );

      store.debugEvent(run('run_started'));
      expect(
        packages.removeBlockedReason(claudeCodePackage),
        AgentStore.removeBlockedWhileRunning,
      );
      await packages.remove(claudeCodePackage);
      expect(backend.removeCalls, isEmpty, reason: 'the reply is not cut off');
      expect(store.hostInfo, isNotNull, reason: 'the agent stays open');

      store.debugEvent(run('run_finished'));
      expect(packages.removeBlockedReason(claudeCodePackage), isNull);
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
