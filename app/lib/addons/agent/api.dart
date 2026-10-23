// Public surface of the agent add-on. Code outside `addons/agent/` imports
// this file only. Files not exported here are private to the add-on.

export 'add_to_note.dart';
export 'agent_cells.dart';
export 'agent_chat.dart';
export 'agent_composer.dart';
export 'agent_markdown.dart';
export 'agent_mcp_settings.dart';
export 'agent_outline.dart';
export 'agent_outline_view.dart';
export 'agent_panel.dart';
export 'agent_screen.dart';
export 'agent_search_source.dart';
export 'agent_search_vault.dart';
export 'agent_settings_body.dart';
export 'agent_store.dart';
export 'agent_transcript.dart';
export 'agent_turn.dart';
export 'blocks/chat_ref.dart';
export 'chat_reference.dart';
export 'chat_reference_insert.dart';
export 'chat_reference_picker.dart';
export 'chat_reference_resolution.dart';
export 'chat_reference_widgets.dart';
export 'dev_replay.dart';
export 'host_relay_link.dart' show HostRelayLink;
export 'host_relay_status.dart';
export 'mobile_agent_settings.dart';
export 'navigation.dart';
export 'outline_names.dart';


import '../../registry/addon_registry.dart';
import '../../registry/block_codecs.dart';
import '../../services/addon_services.dart';
import '../../state/app_state.dart';
import 'agent_panel.dart';
import 'agent_screen.dart';
import 'agent_store.dart';
import 'agent_settings_body.dart';
import 'mobile_agent_settings.dart';
import 'blocks/chat_ref.dart';
import 'commands.dart';
import 'chat_type.dart';
import 'chat_search.dart';
export 'chat_search.dart'
    show AgentChatSearchSource, ChatSearchTarget, openChatHit;
export 'chat_type.dart' show chatNoteType, chatChip, chatCard, chatEmbed;
import 'navigation.dart';

class AgentAddon extends Addon {
  const AgentAddon();
  @override
  String get id => 'agent';
  @override
  List<String> get dependsOn => const [];
  @override
  void register(AddonContext ctx) {
    ctx.registerNoteType(chatNoteType);
    ctx.searchSources.register(const AgentChatSearchSource());
    ctx.blockKinds.register(
      chatRefBlock.inGroup(
        chatRefBlock.group,
        codecs: genericBlockCodecs,
        body: (block) => renderChatRef(block, ctx.noteTypes),
      ),
    );
    ctx.slashItems.register(
      SlashItem(
        id: 'chat_ref',
        label: 'Chat',
        description: 'A chapter or section of a chat.',
        blockKind: 'chat_ref',
        run: (_) {},
      ),
    );
    registerAgentCommands(ctx.commands);
    ctx.shellPanels.register(
      AddonShellPanel(
        id: 'agent',
        label: 'Agent',
        routeName: agentRouteName,
        isEnabled: (context) {
          final app = AppScope.maybeOf(context);
          return app != null && AgentStore.of(app).enabled;
        },
        build: (_) => const AgentScreen(),
        wrap: (_, child) => AgentPanelShell(child: child),
        headerAction: (_) => const AgentPanelToggle(),
      ),
    );
    ctx.settingsSections.register(
      AddonSettingsSection(
        id: 'agent.desktop',
        label: 'Agent',
        location: 'ai',
        placement: 'top',
        isVisible: (_) => AgentStore.canRun && !hubAgentPlatform,
        build: (context) => AgentSettingsSection(
          store: AgentStore.of(context.app),
          onChanged: () => ctx.services.pumpAi(context),
        ),
      ),
    );
    ctx.settingsSections.register(
      AddonSettingsSection(
        id: 'agent.mobile',
        label: 'Agent',
        location: 'ai',
        placement: 'beforeTasks',
        isVisible: (_) => hubAgentPlatform,
        build: (context) =>
            MobileAgentSettings(store: AgentStore.of(context.app)),
      ),
    );
  }
}
