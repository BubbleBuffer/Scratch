// Settings > AI > Runner names: the computers that run tasks are listed by
// their vault names, the hub is not, and Rename changes one through the runner
// bridge. The bridge is a fake (AiStore.runnerBridgeOverride), so no vault
// call is made for the list or the rename.
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/ai/ai_store.dart';
import 'package:notes_app/src/rust/api/ai.dart';

import 'harness.dart';
import 'sync_helpers.dart';

Finder key(String k) => find.byKey(ValueKey(k));

/// Answers the runner list from [names] and records the renames.
class _FakeBridge implements AiRunnerBridge {
  final names = <String, String>{
    'studio': 'Studio-PC',
    'laptop': 'Laptop',
  };
  final calls = <String>[];

  AiRunnerList _list() => AiRunnerList(
    defaultRunnerId: 'studio',
    runners: [
      AiRunner(
        projects: const [],
        id: 'hub',
        name: 'Hub',
        kind: AiRunnerKind.hub,
        online: true,
        lastSeenMs: null,
        isThisDevice: false,
      ),
      AiRunner(
        projects: const [],
        id: 'studio',
        name: names['studio']!,
        kind: AiRunnerKind.pc,
        online: true,
        lastSeenMs: null,
        isThisDevice: true,
      ),
      AiRunner(
        projects: const [],
        id: 'laptop',
        name: names['laptop']!,
        kind: AiRunnerKind.pc,
        online: false,
        lastSeenMs: null,
        isThisDevice: false,
      ),
    ],
  );

  @override
  Future<AiRunnerList> listRunners({required bool hubUsable}) async {
    calls.add('list:$hubUsable');
    return _list();
  }

  @override
  Future<AiTask> assign({
    required String prompt,
    required String page,
    required String runnerId,
    String? projectId,
  }) => throw UnimplementedError('not used in Settings');

  @override
  Future<void> rename({required String deviceId, required String name}) async {
    calls.add('rename:$deviceId:$name');
    names[deviceId] = name;
  }
}

Future<void> _openAiSettings(WidgetTester t) async {
  await tapAndSettle(t, key('sidebar-settings'));
  await tapAndSettle(t, key('settings-nav-ai'));
  await real(t, () => Future<void>.delayed(const Duration(milliseconds: 50)));
  await settle(t);
}

void main() {
  setUpAll(initCore);

  appTest('Settings > AI lists the computers without the hub, and Rename changes a name', (
    t,
  ) async {
    final bridge = _FakeBridge();
    AiStore.runnerBridgeOverride = (_) => bridge;
    addTearDown(() => AiStore.runnerBridgeOverride = null);
    await pumpApp(t, mobile: false);
    await _openAiSettings(t);

    expect(key('runner-name-studio'), findsOneWidget);
    expect(key('runner-name-laptop'), findsOneWidget);
    expect(key('runner-name-hub'), findsNothing);

    await t.ensureVisible(key('runner-rename-laptop'));
    await tapAndSettle(t, key('runner-rename-laptop'));
    await t.enterText(key('runner-name-field-laptop'), '  Desk  ');
    await tapAndSettle(t, key('runner-name-save-laptop'));

    expect(bridge.calls, contains('rename:laptop:Desk'));
    expect(key('runner-name-field-laptop'), findsNothing);
    expect(key('runner-name-laptop'), findsOneWidget);
  });

  appTest('Settings > AI refuses an empty name and renames nothing', (t) async {
    final bridge = _FakeBridge();
    AiStore.runnerBridgeOverride = (_) => bridge;
    addTearDown(() => AiStore.runnerBridgeOverride = null);
    await pumpApp(t, mobile: false);
    await _openAiSettings(t);

    await t.ensureVisible(key('runner-rename-laptop'));
    await tapAndSettle(t, key('runner-rename-laptop'));
    await t.enterText(key('runner-name-field-laptop'), '   ');
    await tapAndSettle(t, key('runner-name-save-laptop'));

    expect(bridge.calls.where((c) => c.startsWith('rename:')), isEmpty);
    expect(find.text('Enter a name.'), findsOneWidget);
    expect(key('runner-name-field-laptop'), findsOneWidget);
  });
}
