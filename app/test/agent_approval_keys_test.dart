// With the chat on screen and in the dock at once, the approval keys 1, 2 and
// 3 answer through one bar, never two.
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';

import 'agent_chat_partb_test.dart' show showWidget;
import 'agent_test.dart' show openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);

  appTest('two chat panes: key 1 decides once, not once per pane', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    final session = (await real(
      tester,
      () => store.createSession(title: 'Two panes'),
    ))!;
    AgentApproval approval(String id) => AgentApproval(
      id: id,
      sessionId: session.id,
      runId: 'run',
      callId: 'call-$id',
      tool: 'delete_page',
      argumentsJson: '{"page":"Garden plan"}',
      state: 'pending',
      createdAt: '2026-10-06T11:20:00Z',
    );
    await showWidget(
      tester,
      Column(
        children: [
          Expanded(child: AgentChat(store: store, panel: true)),
          Expanded(child: AgentChat(store: store, panel: true)),
        ],
      ),
    );
    store.debugAddApproval(approval('a1'));
    store.debugAddApproval(approval('a2'));
    await settle(tester);
    expect(find.text('Needs approval'), findsNWidgets(4));

    // Both panes show a1 first and both would answer it. The one decision
    // removes a1 from both; the second would have asked the real agent about
    // an approval that is gone and left an error behind.
    await tester.sendKeyEvent(LogicalKeyboardKey.digit1);
    await settle(tester);
    expect(store.error, isNull);
    expect(find.text('Needs approval'), findsNWidgets(2));
  });
}
