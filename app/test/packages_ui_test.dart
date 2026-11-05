// The download card and Settings > Agent > Installed packages, driven by a
// fake bridge (packages_fakes.dart) on the app's themed shell.
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/agent_runtime_card.dart';
import 'package:notes_app/addons/agent/packages_settings.dart';
import 'package:notes_app/addons/agent/packages_store.dart';
import 'package:notes_app/features/settings/settings_kit.dart';
import 'package:notes_app/shell/app_shell.dart';
import 'package:notes_app/state/app_state.dart';

import 'harness.dart';
import 'addons/agent/packages_fakes.dart';

Finder key(String k) => find.byKey(ValueKey(k));

const _name = claudeCodePackage;

/// Pumps [child] on the themed shell, 480 wide.
Future<void> _pump(WidgetTester tester, Widget child) async {
  await tester.pumpWidget(
    NotesShell(
      forceDark: false,
      navigatorKey: GlobalKey<NavigatorState>(),
      wrap: (c) => AppScope(state: current.app, child: c),
      home: Center(child: SizedBox(width: 480, child: child)),
    ),
  );
  await tester.pump();
}

/// Lets the store's futures and streams run.
Future<void> _flush(WidgetTester tester) async {
  for (var i = 0; i < 4; i++) {
    await tester.pump();
  }
}

String _statusText(WidgetTester tester) =>
    tester.widget<Text>(key('agent-runtime-status')).data!;

/// The card on a store, with the host's facts in notifiers the test flips.
class _CardHost extends StatelessWidget {
  const _CardHost({
    required this.packages,
    required this.selected,
    required this.available,
  });
  final PackagesStore packages;
  final ValueNotifier<bool> selected;
  final ValueNotifier<bool?> available;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([packages, selected, available]),
    builder: (_, _) => RuntimePackageCard(
      packages: packages,
      selected: selected.value,
      available: available.value,
    ),
  );
}

void main() {
  setUpAll(initCore);

  Future<(PackagesStore, FakePackagesBackend, ValueNotifier<bool?>)> card(
    WidgetTester tester,
    List<PackageInfo> packages, {
    bool? available = false,
    bool selected = true,
  }) async {
    await pumpApp(tester, mobile: false);
    final backend = FakePackagesBackend(packages);
    final store = PackagesStore(backend: backend);
    await store.refresh();
    final avail = ValueNotifier<bool?>(available);
    await _pump(
      tester,
      _CardHost(
        packages: store,
        selected: ValueNotifier(selected),
        available: avail,
      ),
    );
    addTearDown(store.dispose);
    return (store, backend, avail);
  }

  appTest('idle: the name, the size and Download', (tester) async {
    await card(tester, [claudeNotInstalled()]);
    expect(key('agent-runtime-card'), findsOneWidget);
    expect(find.text('Claude Code'), findsOneWidget);
    expect(_statusText(tester), '168 MB');
    expect(find.text('Download'), findsOneWidget);
  });

  appTest('Download shows the percent, the steps, then Ready', (tester) async {
    final (store, backend, avail) = await card(tester, [claudeNotInstalled()]);
    await tester.tap(key('agent-runtime-action'));
    await _flush(tester);
    expect(backend.installCalls, [_name]);
    expect(find.text('Download'), findsNothing);
    expect(_statusText(tester), 'Downloading 0%');

    backend.lastInstall.add(downloading(42, 100));
    await _flush(tester);
    expect(_statusText(tester), 'Downloading 42%');

    for (final (phase, text) in [
      (PackagePhase.verifying, 'Verifying'),
      (PackagePhase.extracting, 'Extracting'),
      (PackagePhase.installing, 'Installing'),
    ]) {
      backend.lastInstall.add(step(phase));
      await _flush(tester);
      expect(_statusText(tester), text);
    }

    backend.state[_name] = claudeInstalled();
    await backend.lastInstall.close();
    await _flush(tester);
    expect(store.justInstalled(_name), isTrue);
    expect(_statusText(tester), 'Ready');

    // Detection finds Claude Code: the card goes.
    avail.value = true;
    await _flush(tester);
    expect(key('agent-runtime-card'), findsNothing);
    expect(key('agent-runtime-card-compact'), findsNothing);
  });

  appTest('an error shows the message and Retry', (tester) async {
    final (store, backend, _) = await card(tester, [claudeNotInstalled()]);
    await tester.tap(key('agent-runtime-action'));
    await _flush(tester);
    backend.lastInstall.addError('not enough disk space');
    await _flush(tester);
    expect(_statusText(tester), 'not enough disk space');
    expect(find.text('Retry'), findsOneWidget);

    await tester.tap(key('agent-runtime-action'));
    await _flush(tester);
    expect(backend.installCalls, [_name, _name]);
    expect(_statusText(tester), 'Downloading 0%');
    expect(store.statusOf(_name), isA<PackageInstalling>());
  });

  appTest('an update while Claude Code works is one compact line', (
    tester,
  ) async {
    await card(tester, [claudeUpdate()], available: true);
    expect(key('agent-runtime-card'), findsNothing);
    expect(key('agent-runtime-card-compact'), findsOneWidget);
    expect(_statusText(tester), 'Update available · 168 MB');
    expect(find.text('Update'), findsOneWidget);
  });

  appTest('hidden when Claude Code works and is up to date', (tester) async {
    await card(tester, [claudeInstalled()], available: true);
    expect(key('agent-runtime-card'), findsNothing);
    expect(key('agent-runtime-card-compact'), findsNothing);
  });

  appTest('hidden when the model is not from Claude Code', (tester) async {
    await card(tester, [claudeNotInstalled()], selected: false);
    expect(key('agent-runtime-card'), findsNothing);
  });

  appTest('hidden where the build lists no package', (tester) async {
    await card(tester, const []);
    expect(key('agent-runtime-card'), findsNothing);
  });

  group('Installed packages', () {
    Future<(PackagesStore, FakePackagesBackend)> section(
      WidgetTester tester,
      List<PackageInfo> packages,
    ) async {
      await pumpApp(tester, mobile: false);
      final backend = FakePackagesBackend(packages);
      final store = PackagesStore(backend: backend);
      addTearDown(store.dispose);
      await _pump(tester, PackagesSettings(packages: store));
      await _flush(tester);
      return (store, backend);
    }

    appTest('lists an installed package with version, size and Remove', (
      tester,
    ) async {
      final (_, backend) = await section(tester, [claudeInstalled()]);
      expect(find.text('Installed packages'), findsOneWidget);
      expect(find.text('Claude Code'), findsOneWidget);
      expect(find.text('Version 2.0.0 · 412 MB'), findsOneWidget);
      expect(key('package-install-$_name'), findsNothing);
      await tester.tap(key('package-remove-$_name'));
      await _flush(tester);
      expect(backend.removeCalls, [_name]);
      expect(find.text('Not installed · 168 MB download'), findsOneWidget);
      expect(key('package-remove-$_name'), findsNothing);
      expect(key('package-install-$_name'), findsOneWidget);
    });

    appTest('shows nothing when the list is empty', (tester) async {
      await section(tester, const []);
      expect(find.text('Installed packages'), findsNothing);
      expect(find.byType(SList), findsNothing);
    });

    appTest('shows nothing when no package is available here', (tester) async {
      await section(tester, [
        PackageInfo(
          name: _name,
          version: '',
          downloadBytes: 0,
          state: PackageState.notAvailable,
        ),
      ]);
      expect(find.text('Installed packages'), findsNothing);
    });

    appTest('an available package has Download with its size', (tester) async {
      final (store, backend) = await section(tester, [claudeNotInstalled()]);
      expect(find.text('Not installed · 168 MB download'), findsOneWidget);
      expect(key('package-remove-$_name'), findsNothing);
      await tester.tap(key('package-install-$_name'));
      await _flush(tester);
      expect(backend.installCalls, [_name]);
      expect(find.text('Downloading 0%'), findsOneWidget);

      // The card and this list share one store: progress shows here too.
      backend.lastInstall.add(downloading(50, 100));
      await _flush(tester);
      expect(find.text('Downloading 50%'), findsOneWidget);
      expect(store.installing, isTrue);

      backend.state[_name] = claudeInstalled();
      await backend.lastInstall.close();
      await _flush(tester);
      expect(find.text('Version 2.0.0 · 412 MB'), findsOneWidget);
    });

    appTest('an update has Update and Remove; Remove waits for the install', (
      tester,
    ) async {
      final (store, backend) = await section(tester, [claudeUpdate()]);
      expect(
        find.text('Version 2.0.0 · 412 MB · update to 2.1.0, 168 MB'),
        findsOneWidget,
      );
      expect(find.text('Update'), findsOneWidget);
      expect(find.text('Remove'), findsOneWidget);
      expect(
        tester.widget<SButton>(key('package-remove-$_name')).onTap,
        isNotNull,
      );
      await tester.tap(key('package-install-$_name'));
      await _flush(tester);
      expect(store.installing, isTrue);
      expect(
        tester.widget<SButton>(key('package-remove-$_name')).onTap,
        isNull,
      );
      expect(
        tester.widget<SButton>(key('package-install-$_name')).onTap,
        isNull,
        reason: 'one install at a time',
      );
      backend.lastInstall.addError('download failed, check the connection: x');
      await _flush(tester);
      expect(
        find.text('download failed, check the connection: x'),
        findsOneWidget,
      );
      expect(
        tester.widget<SButton>(key('package-install-$_name')).onTap,
        isNotNull,
        reason: 'retry',
      );
    });
  });
}
