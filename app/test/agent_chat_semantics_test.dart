// The agent chat's semantics tree as the app remote lists it
// (`scratch-cli app tree`): the composer is the one text field of the chat,
// with tap and focus while unfocused, and the chat filter is a text field.
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/remote/semantics_driver.dart';

import 'agent_chat_partb_test.dart' show showWidget;
import 'agent_test.dart' show openAgentScreen, openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

List<UiNode> _fields(WidgetTester tester) =>
    SemanticsDriver(nextFrame: () async => tester.pump())
        .snapshot()
        .where((n) => n.role == 'textfield')
        .toList();

void main() {
  setUpAll(initCore);

  appTest('the chat has one text field: the composer, with tap and focus', (
    tester,
  ) async {
    final handle = tester.ensureSemantics();
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Semantics'));
    await showWidget(tester, ScratchChatPane(store: store));

    final fields = _fields(tester);
    expect(fields, hasLength(1));
    final composer = fields.single;
    expect(composer.label, 'Message to the agent');
    expect(composer.hint, 'Ask the agent…');
    expect(composer.focused, isFalse);
    expect(composer.actions, containsAll(['tap', 'focus']));
    handle.dispose();
  });

  appTest('the chat filter is a text field with tap and focus', (tester) async {
    final handle = tester.ensureSemantics();
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Semantics'));
    await openAgentScreen(tester);
    await settle(tester);

    final filter = _fields(tester).where((n) => n.label == 'Filter chats');
    expect(filter, hasLength(1));
    expect(filter.single.actions, containsAll(['tap', 'focus']));
    // The composer is still the only other text field on the screen.
    expect(_fields(tester), hasLength(2));
    handle.dispose();
  });

  appTest('focusing the composer from the tree lets it take text', (
    tester,
  ) async {
    final handle = tester.ensureSemantics();
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Semantics'));
    await showWidget(tester, ScratchChatPane(store: store));

    final driver = SemanticsDriver(nextFrame: () async => tester.pump());
    final id = _fields(tester).single.id;
    await driver.focus(id);
    await tester.pump();
    expect(_fields(tester).single.focused, isTrue);
    expect(_fields(tester).single.actions, contains('setText'));
    await tester.pump();
    await driver.type(id, 'hello', replace: true);
    await tester.pump();
    expect(_fields(tester), hasLength(1));
    expect(_fields(tester).single.value, 'hello');
    handle.dispose();
  });
}
