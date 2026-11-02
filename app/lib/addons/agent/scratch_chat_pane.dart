// The chat the Agent section and the editor dock show: the kit's
// `AgentChatPane` with Scratch's transcript (`ScratchChatBody`), composer and
// hooks in its slots. The pane brings the dock: approval bars, the error
// line and the composer's state; its commands go through `AgentStore`.
//
// While there is no backend yet (the agent is opening, or off), and when the
// chat asks to sign in, the same shell shows Scratch's own body and composer
// without the pane (`_Shell`): the sign-in prompt, the off and error lines
// live in `ScratchChatBody`, and the composer is Scratch's without a pane
// slot. Sign-in has no composer.

import 'package:agent_kit/agent_kit.dart';
import 'package:bb_ui/bb_ui.dart' as bb show FormFactor;
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'agent_approval_bars.dart';
import 'agent_composer.dart' as scratch;
import 'agent_runtime_card.dart';
import 'agent_store.dart';
import 'kit_backend.dart';
import 'kit_commands.dart';
import 'kit_hooks.dart';
import 'scratch_chat_body.dart';

class ScratchChatPane extends StatefulWidget {
  const ScratchChatPane({
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
  State<ScratchChatPane> createState() => _ScratchChatPaneState();
}

class _ScratchChatPaneState extends State<ScratchChatPane> {
  ScratchKitBackend? _backend;
  Object? _agent;
  int _generation = 0;

  @override
  void initState() {
    super.initState();
    widget.store.addListener(_sync);
    _sync();
  }

  @override
  void dispose() {
    widget.store.removeListener(_sync);
    super.dispose();
  }

  /// Follows the store: a new agent (the host was switched) gets a new
  /// backend.
  Future<void> _sync() async {
    final store = widget.store;
    final generation = ++_generation;
    final agent = await store.ensureOpen();
    if (!mounted || generation != _generation) return;
    if (agent == null) {
      if (_backend != null) {
        setState(() {
          _agent = null;
          _backend = null;
        });
      }
      return;
    }
    if (identical(agent, _agent)) return;
    setState(() {
      _agent = agent;
      _backend = ScratchKitBackend(
        NotesAgentSource(agent, events: store.eventsFor(agent)),
        commands: StoreKitCommands(store),
        approvals: false,
      );
    });
  }

  Widget _shell() =>
      _Shell(store: widget.store, panel: widget.panel, title: widget.title);

  @override
  Widget build(BuildContext context) {
    final backend = _backend;
    if (backend == null) return _shell();
    return ListenableBuilder(
      listenable: widget.store,
      builder: (context, _) =>
          widget.store.requiresHubSignIn ? _shell() : _pane(context, backend),
    );
  }

  Widget _pane(BuildContext context, ScratchKitBackend backend) {
    final store = widget.store;
    final c = context.colors;
    final panel = widget.panel;
    final pad = panel ? 12.0 : 32.0;
    final title = widget.title;
    return Container(
      decoration: BoxDecoration(
        color: c.bgPrimary,
        borderRadius: Radii.panel,
        boxShadow: panel ? null : c.seam,
      ),
      child: ClipRRect(
        borderRadius: Radii.panel,
        child: BbThemeScope(
          brightness: c.dark ? Brightness.dark : Brightness.light,
          formFactor: context.mobile
              ? bb.FormFactor.mobile
              : bb.FormFactor.desktop,
          child: AgentChatPane(
            key: const ValueKey('agent-kit-pane'),
            backend: backend,
            // No open chat is a draft: the pane makes the chat on the first
            // send.
            chatId: store.openId,
            onChatCreated: (id) {
              if (store.openId != id) store.openSession(id);
            },
            contentWidth: Layout.contentWidth,
            showCrewStrip: false,
            showHeader: !panel && title != null,
            headerBuilder: (context, _) => Padding(
              padding: const EdgeInsets.fromLTRB(24, 12, 24, 10),
              child: Text(
                title ?? '',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: tsDisplay(context.type.h3, c.textPrimary),
              ),
            ),
            metrics: PaneMetrics(
              dockPadding: EdgeInsets.only(bottom: panel ? 12 : 20),
              dockGutter: pad,
            ),
            transcriptActions: scratchTranscriptActions(context, store),
            transcriptBuilder: (_, _, _) =>
                ScratchChatBody(store: store, panel: panel),
            composerBuilder: (_, slot) => Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                // Claude Code is a download: the card offers it (nothing
                // when it is installed or the model is not from it).
                AgentRuntimeCard(store: store),
                if (store.pendingApprovals.isNotEmpty)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 8),
                    child: AgentApprovalBars(
                      store: store,
                      draft: slot.controller,
                    ),
                  ),
                scratch.AgentComposer(store: store, panel: panel, slot: slot),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// The chat without the pane: Scratch's body, approval bars and composer in
/// the pane's frame. Shows while there is no backend and when signing in.
class _Shell extends StatefulWidget {
  const _Shell({required this.store, required this.panel, this.title});
  final AgentStore store;
  final bool panel;
  final String? title;

  @override
  State<_Shell> createState() => _ShellState();
}

class _ShellState extends State<_Shell> {
  /// The composer's text, so the approval bar knows when 1, 2 and 3 are not
  /// typing.
  final _draft = TextEditingController();

  @override
  void dispose() {
    _draft.dispose();
    super.dispose();
  }

  Widget _approvalBars(AgentStore store, double pad) {
    if (store.pendingApprovals.isEmpty) return const SizedBox.shrink();
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: Layout.contentWidth + 64),
        child: Padding(
          padding: EdgeInsets.fromLTRB(pad, 0, pad, 8),
          child: AgentApprovalBars(store: store, draft: _draft),
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
                        child: scratch.AgentComposer(
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
