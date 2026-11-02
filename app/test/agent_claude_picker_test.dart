// The model picker lists "Claude Code, Not installed" while the package is
// missing; picking it shows the download card and sends nothing.
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/addons/agent/packages_store.dart';
import 'package:notes_app/shell/app_shell.dart';
import 'package:notes_app/src/rust/api/agent.dart';
import 'package:notes_app/state/app_state.dart';

import 'addons/agent/packages_fakes.dart';
import 'agent_test.dart' show fakeExtensionSkipReason, openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

Finder key(String k) => find.byKey(ValueKey(k));

AgentHostInfo _host(List<String> providers) => AgentHostInfo(
  kind: 'local',
  packs: const [],
  tools: const [],
  providers: providers,
  canRunTasks: true,
  version: '0',
  permissionMode: AgentPermissionMode.bypass,
  projects: false,
);

void main() {
  setUpAll(initCore);

  group('Claude Code in the picker', () {
    appTest('listed as Not installed; picking it shows the card', (
      tester,
    ) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      final backend = FakePackagesBackend([claudeNotInstalled()]);
      final packages = PackagesStore(backend: backend);
      addTearDown(packages.dispose);
      await real(tester, packages.refresh);
      store.usePackages(packages);
      await real(tester, () => store.createSession(title: 'Picker'));
      // The host does not report Claude Code; the chat uses another model.
      store.hostInfo = _host(const []);
      store.pickModel(store.openId, 'other/model');
      expect(store.shownModels(), contains(claudeCodePlaceholderModel));

      await tester.pumpWidget(
        NotesShell(
          forceDark: false,
          navigatorKey: GlobalKey<NavigatorState>(),
          wrap: (child) => AppScope(state: current.app, child: child),
          home: Center(
            child: SizedBox(
              width: 480,
              height: 650,
              child: ScratchChatPane(store: store),
            ),
          ),
        ),
      );
      await settle(tester);
      expect(key('agent-runtime-card'), findsNothing);

      // The pick shows the chat's current model; opening it lists the
      // stand-in as an option. (bb_ui's SelectButton renders labels only, so
      // the "Not installed" detail is asserted on the store.)
      await tester.tap(find.text(store.modelLabel('other/model')));
      await settle(tester);
      expect(store.modelDetail(claudeCodePlaceholderModel), 'Not installed');
      await tester.tap(key('option-Claude Code'));
      await settle(tester);

      expect(store.modelOf(store.openId), claudeCodePlaceholderModel);
      expect(key('agent-runtime-card'), findsOneWidget);
      expect(await real(tester, () => store.send('hello')), isNull);
      expect(key('agent-runtime-card'), findsOneWidget);
    });

    appTest('not listed once Claude Code is detected, or off this computer', (
      tester,
    ) async {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      final packages = PackagesStore(
        backend: FakePackagesBackend([claudeNotInstalled()]),
      );
      addTearDown(packages.dispose);
      await real(tester, packages.refresh);
      store.usePackages(packages);
      store.hostInfo = _host(const ['claude-code']);
      expect(store.shownModels(), isNot(contains(claudeCodePlaceholderModel)));
      store.hostInfo = _host(const []);
      expect(store.offersClaudeCode, isTrue);
      expect(store.modelDetail(claudeCodePlaceholderModel), 'Not installed');

      // Where the build lists no package there is nothing to offer.
      final none = PackagesStore(backend: FakePackagesBackend(const []));
      addTearDown(none.dispose);
      await real(tester, none.refresh);
      store.usePackages(none);
      expect(store.offersClaudeCode, isFalse);
    });
  }, skip: fakeExtensionSkipReason);
}
