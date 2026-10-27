// Dev pane: the shared agent kit's chat pane over Scratch's agent, read-only.
//
// Off unless the app is built with `--dart-define=AGENT_KIT_PANE=true`; then
// the Agent screen shows this pane instead of the old chat. The old screen is
// untouched and stays the default. The pane draws the store's open chat; the
// kit gets its look from `BbThemeScope`, fed from Scratch's theme.

import 'package:agent_kit/agent_kit.dart';
import 'package:bb_ui/bb_ui.dart' as bb show FormFactor;
import 'package:flutter/widgets.dart';

import '../../services/addon_services.dart';
import '../../state/app_state.dart';
import '../../ui/ui.dart';
import 'agent_store.dart';
import 'kit_backend.dart';

/// Built with `--dart-define=AGENT_KIT_PANE=true`.
const agentKitPane = bool.fromEnvironment('AGENT_KIT_PANE');

class AgentKitScreen extends StatefulWidget {
  const AgentKitScreen({super.key});

  @override
  State<AgentKitScreen> createState() => _AgentKitScreenState();
}

class _AgentKitScreenState extends State<AgentKitScreen> {
  late final AgentStore store = AgentStore.of(context.app);
  ScratchKitBackend? _backend;
  Object? _agent;

  @override
  void initState() {
    super.initState();
    store.addListener(_sync);
    _sync();
  }

  @override
  void dispose() {
    store.removeListener(_sync);
    super.dispose();
  }

  /// Follows the store: a new agent (the host was switched) gets a new
  /// backend, and an unopened store opens its newest chat.
  Future<void> _sync() async {
    final agent = await store.ensureOpen();
    if (!mounted || agent == null) return;
    if (store.openId == null && store.sessions.isNotEmpty) {
      await store.openSession(store.sessions.first.id);
    }
    if (!mounted || identical(agent, _agent)) return;
    setState(() {
      _agent = agent;
      _backend = ScratchKitBackend(NotesAgentSource(agent));
    });
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return ColoredBox(
      key: const ValueKey('agent-kit-screen'),
      color: c.desk,
      child: Row(
        children: [
          if (!context.mobile) const Sidebar(agentOpen: true),
          Expanded(
            child: BbThemeScope(
              brightness: c.dark ? Brightness.dark : Brightness.light,
              formFactor: context.mobile
                  ? bb.FormFactor.mobile
                  : bb.FormFactor.desktop,
              child: ListenableBuilder(
                listenable: store,
                builder: (context, _) {
                  final backend = _backend, chat = store.openId;
                  if (backend == null || chat == null) {
                    return const SizedBox.expand();
                  }
                  return AgentChatPane(backend: backend, chatId: chat);
                },
              ),
            ),
          ),
        ],
      ),
    );
  }
}
