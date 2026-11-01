
import 'package:agent_kit/agent_kit.dart' show ApprovalBar, ApprovalKeyPane;
import 'package:agent_kit_core/agent_kit_core.dart' show ApprovalChoice;
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'agent_composer.dart';
import 'agent_store.dart';
import 'kit_backend.dart' show kitApproval;
import 'scratch_chat_body.dart';

export 'scratch_chat_body.dart' show agentEmptyLine;

/// The same transcript and composer serve the Agent section and editor dock.
class AgentChat extends StatefulWidget {
  const AgentChat({
    super.key,
    required this.store,
    this.panel = false,
    this.title,
  });
  final AgentStore store;
  final bool panel;

  /// The full-screen host's header. The dock supplies its own header.
  final String? title;
  @override
  State<AgentChat> createState() => _AgentChatState();
}

class _AgentChatState extends State<AgentChat> {
  /// The composer's text, so the approval bar knows when 1, 2 and 3 are
  /// not typing.
  final _draft = TextEditingController();

  @override
  void dispose() {
    _draft.dispose();
    super.dispose();
  }

  /// The requests the agent waits on, above the composer: the first one
  /// answers to 1, 2 and 3.
  Widget _approvalBars(AgentStore store, double pad) {
    final pending = store.pendingApprovals;
    if (pending.isEmpty) return const SizedBox.shrink();
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: Layout.contentWidth + 64),
        child: Padding(
          padding: EdgeInsets.fromLTRB(pad, 0, pad, 8),
          child: Column(
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
                    composerEmpty: () => _draft.text.trim().isEmpty,
                    onDecide: (d) => store.decide(
                      pending[i].id,
                      switch (d.choice) {
                        ApprovalChoice.approve => 'approve',
                        ApprovalChoice.approveForSession => 'approve_session',
                        ApprovalChoice.deny => 'deny',
                      },
                      reason: d.reason,
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => ApprovalKeyPane(
    child: ListenableBuilder(
      listenable: widget.store,
      builder: (context, _) {
        final store = widget.store;
        final c = context.colors;
        final signIn = store.requiresHubSignIn;
        final pad = widget.panel ? 12.0 : 32.0;
        return Container(
          decoration: BoxDecoration(
            color: c.bgPrimary,
            borderRadius: Radii.panel,
            boxShadow: widget.panel ? null : c.seam,
          ),
          child: ClipRRect(
            borderRadius: Radii.panel,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (!widget.panel && widget.title != null)
                  Padding(
                    padding: const EdgeInsets.fromLTRB(24, 12, 24, 10),
                    child: Text(
                      widget.title!,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: tsDisplay(context.type.h3, c.textPrimary),
                    ),
                  ),
                Expanded(
                  child: ScratchChatBody(store: store, panel: widget.panel),
                ),
                if (!signIn) _approvalBars(store, pad),
                if (!signIn)
                  Center(
                    child: ConstrainedBox(
                      constraints: const BoxConstraints(
                        maxWidth: Layout.contentWidth + 64,
                      ),
                      child: Padding(
                        padding: EdgeInsets.fromLTRB(
                          pad,
                          0,
                          pad,
                          widget.panel ? 12 : 20,
                        ),
                        child: AgentComposer(
                          store: store,
                          panel: widget.panel,
                          controller: _draft,
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
        );
      },
    ),
  );
}
