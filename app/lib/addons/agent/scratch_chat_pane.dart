// The chat the Agent section and the editor dock show: the kit's
// `AgentChatPane` with Scratch's transcript (`ScratchChatBody`), composer and
// hooks in its slots. The pane brings the dock: approval bars, the error
// line and the composer's state; its commands go through `AgentStore`.
//
// `--dart-define=AGENT_KIT_PANE=false` builds the old `AgentChat` instead.
// The old chat is also what shows while there is no backend yet, and when the
// chat asks to sign in (no dock).

import 'package:agent_kit/agent_kit.dart';
import 'package:bb_ui/bb_ui.dart' as bb show FormFactor;
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'agent_approval_bars.dart';
import 'agent_chat.dart';
import 'agent_composer.dart' as scratch;
import 'agent_store.dart';
import 'kit_backend.dart';
import 'kit_commands.dart';
import 'kit_hooks.dart';
import 'scratch_chat_body.dart';

/// On unless built with `--dart-define=AGENT_KIT_PANE=false`.
const agentKitPane = bool.fromEnvironment('AGENT_KIT_PANE', defaultValue: true);

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
    if (!agentKitPane) return;
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

  AgentChat _old() =>
      AgentChat(store: widget.store, panel: widget.panel, title: widget.title);

  @override
  Widget build(BuildContext context) {
    final backend = _backend;
    if (!agentKitPane || backend == null) return _old();
    return ListenableBuilder(
      listenable: widget.store,
      builder: (context, _) =>
          widget.store.requiresHubSignIn ? _old() : _pane(context, backend),
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
