import 'package:flutter/widgets.dart';

import '../core/change_feed.dart';
import '../core/vault.dart';
import '../features/settings/settings_store.dart';
import '../state/app_state.dart';
import '../state/editor_focus.dart';
import '../features/ai/ai_settings.dart';
import '../features/ai/ai_store.dart';
import '../registry/addon_registry.dart';
import '../shell/page_stack.dart';

// Transitional host UI contracts. Features move behind their own add-on APIs
// later; an add-on never imports these host implementations directly.
export '../features/ai/ai_settings.dart';
export '../features/ai/ai_store.dart'
    show AgentRuns, AiRunnerBridge, AiStatus, AiTask, AiTaskSource;
export '../features/settings/settings_store.dart' show AppSettings;
export '../features/settings/controls.dart'
    show
        SettingsHeading,
        SettingsRow,
        SettingsField,
        SelectButton,
        Toggle,
        OutlineButton,
        CopyableText;
export '../features/settings/settings_kit.dart';
export '../features/settings/settings_screen.dart'
    show showSettings, openSettingsSection, SettingsSection;
export '../features/sync/hub_auth.dart' show HubAuthService;
export '../features/sync/sync_settings.dart' show HubSignInSettings;
export '../features/sync/sync_indicator.dart' show SyncIndicator;
export '../editor/mention_menu.dart'
    show MentionItem, MentionMenu, mentionPages;
export '../editor/rich_controller.dart' show markedSpan;
export '../shell/sidebar.dart'
    show Sidebar, SidebarReopenButton, CollapsedFeedbackButton;
export '../shell/home.dart' show ShellMobileFrame;
export '../shell/page_stack.dart' show PageStackScope, PageStackNavigator;
export '../shell/navigation.dart' show openPage;

/// The host's services, resolved only while an add-on handles a UI action.
/// Registration itself never opens a vault or starts background work.
class AddonServices {
  const AddonServices();
  Vault vault(BuildContext context) => AppScope.of(context).vault;
  ChangeFeed feed(BuildContext context) => AppScope.of(context).feed;
  AppSettings settings(BuildContext context) => AppSettings.read(context);
  AiSettings? aiSettingsOf(Vault vault) => AiStore.settingsOf(vault);
  Future<void> pumpAi(BuildContext context) => AiStore.of(context.app).pump();

  /// Starts and stops the agent's runs (on the synced default runner).
  AgentRuns agentRuns(BuildContext context) => AiStore.of(context.app);

  /// The agent's task rows and runner calls (the kanban runner picker).
  AiTaskSource aiTasks(BuildContext context) => AiStore.of(context.app);

  /// Opens the agent chat of a run's session.
  Future<void> openAgentSession(BuildContext context, String sessionId) async {
    await AiStore.openSession(context, sessionId);
  }

  bool addonEnabled(String id) =>
      activeAddonContext?.installedIds.contains(id) ?? true;
  void closeRoute(BuildContext context, String name) {
    final stack = PageStackScope.of(context);
    final nav = stack.navigator ?? Navigator.of(context);
    for (final route
        in stack.routes.where((r) => r.settings.name == name).toList()) {
      nav.removeRoute(route);
    }
  }

  String? selectedPageId(BuildContext context) {
    final app = AppScope.maybeOf(context);
    return app == null ? null : EditorFocus.forApp(app).pageId;
  }
}
