// The requests the agent waits on, as the kit's approval bars above the
// composer: the first one answers to 1, 2 and 3. They read the store, so the
// old chat and the kit pane's dock (the pane itself draws none: Scratch's
// backend leaves approvals out of its snapshot) show the same bars.

import 'package:agent_kit/agent_kit.dart' show ApprovalBar;
import 'package:agent_kit_core/agent_kit_core.dart' show ApprovalChoice;
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'agent_store.dart';
import 'kit_backend.dart' show kitApproval;

class AgentApprovalBars extends StatelessWidget {
  const AgentApprovalBars({
    super.key,
    required this.store,
    required this.draft,
  });
  final AgentStore store;

  /// The composer's text, so the bar knows when 1, 2 and 3 are not typing.
  final TextEditingController draft;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: store,
    builder: (context, _) {
      final pending = store.pendingApprovals;
      if (pending.isEmpty) return const SizedBox.shrink();
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (var i = 0; i < pending.length; i++)
            Padding(
              padding: EdgeInsets.only(top: i == 0 ? 0 : 6),
              child: ApprovalBar(
                key: ValueKey('agent-approval-${pending[i].id}'),
                approval: kitApproval(pending[i]),
                now: clockNow(),
                captureKeys: i == 0,
                composerEmpty: () => draft.text.trim().isEmpty,
                onDecide: (d) => store.decide(pending[i].id, switch (d.choice) {
                  ApprovalChoice.approve => 'approve',
                  ApprovalChoice.approveForSession => 'approve_session',
                  ApprovalChoice.deny => 'deny',
                }, reason: d.reason),
              ),
            ),
        ],
      );
    },
  );
}
