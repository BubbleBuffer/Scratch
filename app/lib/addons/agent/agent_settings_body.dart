import '../../services/addon_services.dart';
// Settings > AI > Agent (desktop): whether the in-app agent runs, and
// "Run AI on" (the vault's synced runner, [RunAiOnSetting]). Below it only
// what applies to that runner: for the hub, its model (listed by the hub)
// and tool set; for this computer, its model, tool set, computer access and
// projects, the MCP servers ([McpServersSettings]) and the Claude Code
// extension. The MCP servers also show while "Let the hub use this
// computer" (Settings > Sync) shares them. The runner, models and tool sets
// are vault settings (synced); the rest is this computer's
// (ai_settings.json). If the chosen runner isn't reachable here, chats fall
// back to the other one by themselves.

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import '../../core/file_dialogs.dart';
import 'agent_mcp_settings.dart';
import 'agent_runner_settings.dart';
import 'agent_store.dart';

class AgentSettingsSection extends StatefulWidget {
  const AgentSettingsSection({
    super.key,
    required this.store,
    required this.onChanged,
  });
  final AgentStore store;

  /// After a change (the AI store may start running tasks).
  final VoidCallback onChanged;

  @override
  State<AgentSettingsSection> createState() => _AgentSettingsSectionState();
}

class _AgentSettingsSectionState extends State<AgentSettingsSection> {
  late final _bun = TextEditingController(
    text: widget.store.settings.agentBunPath,
  );
  late final _dir = TextEditingController(
    text: widget.store.settings.agentClaudeCodeDir,
  );
  final _projectName = TextEditingController();
  final _projectPath = TextEditingController();
  final _rename = TextEditingController();
  bool _adding = false;
  String? _renaming;
  bool _saving = false;

  @override
  void initState() {
    super.initState();
    // The tab's model list comes from the open agent; Default model lists
    // the default host's, read here when the tab is on the other host.
    widget.store.retryDefaultHostModels();
    widget.store.addListener(_onStore);
    widget.store.refreshVaultDefaults();
    if (widget.store.enabled) widget.store.ensureOpen();
  }

  @override
  void didUpdateWidget(AgentSettingsSection old) {
    super.didUpdateWidget(old);
    if (!identical(old.store, widget.store)) {
      old.store.removeListener(_onStore);
      widget.store.addListener(_onStore);
    }
  }

  void _onStore() {
    if (!mounted) return;
    if (SchedulerBinding.instance.schedulerPhase ==
        SchedulerPhase.persistentCallbacks) {
      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (mounted) setState(() {});
      });
    } else {
      setState(() {});
    }
  }

  @override
  void dispose() {
    widget.store.removeListener(_onStore);
    for (final c in [
      _bun,
      _dir,
      _projectName,
      _projectPath,
      _rename,
    ]) {
      c.dispose();
    }
    super.dispose();
  }

  void _set(void Function(AiSettings s) f) {
    widget.store.settings.update(f);
    widget.onChanged();
    setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final store = widget.store;
    final s = store.settings;
    final c = context.colors;
    Widget field(
      String label,
      String? description,
      TextEditingController ctl,
      String key,
      String placeholder,
      void Function(AiSettings, String) save,
    ) => SettingsRow(
      label: label,
      description: description,
      stack: true,
      trailing: SettingsField(
        fieldKey: ValueKey(key),
        controller: ctl,
        mono: true,
        placeholder: placeholder,
        onChanged: (v) => _set((s) => save(s, v.trim())),
      ),
    );
    Widget note(String text, Key key, {bool problem = false}) => Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Text(
        text,
        key: key,
        style: tsRaw(
          12,
          16,
          400,
          problem ? c.inlineCodeText : c.textSecondary,
        ),
      ),
    );
    final chosen = chosenRunner(store);
    final onHub = chosen == AgentHost.hub;
    final resolved = store.resolvedHost;
    // The MCP servers also serve the hub while it may use this computer.
    final mcp = KeyedSubtree(
      key: mcpServersAnchor,
      child: McpServersSettings(store: store),
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SettingsRow(
          label: 'Use the agent',
          description: 'Chat with the agent in the Agent section.',
          trailing: Toggle(
            key: const ValueKey('agent-enabled-toggle'),
            value: s.agentEnabled,
            onChanged: (v) => _set((s) => s.agentEnabled = v),
          ),
        ),
        if (s.agentEnabled) ...[
          RunAiOnSetting(store: store, onChanged: widget.onChanged),
          if (resolved.fellBack && resolved.wanted == AgentHost.hub)
            note(
              store.hubLink == null
                  ? "Chats run on this computer until this vault syncs with an https:// hub."
                  : "Chats run on this computer for now.",
              const ValueKey('agent-host-fallback'),
            ),
          if (onHub) ...[
            HubModelSetting(store: store, onChanged: widget.onChanged),
            RunnerToolSetSetting(
              store: store,
              runner: AgentHost.hub,
              onChanged: widget.onChanged,
            ),
            if (store.sharesThisComputer) mcp,
          ] else ...[
            ComputerModelSetting(store: store, onChanged: widget.onChanged),
            RunnerToolSetSetting(
              store: store,
              runner: AgentHost.local,
              onChanged: widget.onChanged,
            ),
            SettingsRow(
              label: 'Computer access',
              description: 'Lets the agent work in projects and run commands. Commands are not sandboxed.',
              trailing: Toggle(
                key: const ValueKey('agent-desktop-tools'),
                value: s.desktopTools,
                onChanged: (v) => _set((s) => s.desktopTools = v),
              ),
            ),
            if (s.desktopTools &&
                store.host == AgentHost.local &&
                (store.hostInfo?.projects ?? false))
              _projects(context),
            mcp,
            field(
              'Claude Code: bun program',
              'Optional. Empty uses the bun program found on this computer.',
              _bun,
              'agent-bun-path',
              store.claudeDetection?.bunPath ?? 'bun',
              (s, v) => s.agentBunPath = v,
            ),
            field(
              'Claude Code: extension folder',
              'Optional. Empty uses the Claude Code extension found on this computer.',
              _dir,
              'agent-claude-code-dir',
              store.claudeDetection?.claudeCodeDir ??
                  'BubblePaw/extensions/claude-code',
              (s, v) => s.agentClaudeCodeDir = v,
            ),
            if (store.error != null)
              note(
                store.error!,
                const ValueKey('agent-project-error'),
                problem: true,
              ),
            if (store.host == AgentHost.local && store.openError != null)
              note(
                store.openError!,
                const ValueKey('agent-settings-error'),
                problem: true,
              ),
          ],
        ],
        // Off, or with the hub chosen: the hub may still use this computer.
        if (!s.agentEnabled && store.sharesThisComputer) mcp,
      ],
    );
  }

  Future<void> _pickFolder() async {
    final path = await FileDialogs.instance.pickFolder(
      title: 'Add a project folder',
    );
    if (path == null || !mounted) return;
    _projectPath.text = path;
    if (_projectName.text.trim().isEmpty) {
      _projectName.text =
          path.split(RegExp(r'[/\\]')).where((p) => p.isNotEmpty).lastOrNull ??
          'Project';
    }
    setState(() => _adding = true);
  }

  Future<void> _addProject() async {
    if (_saving ||
        _projectName.text.trim().isEmpty ||
        _projectPath.text.trim().isEmpty) {
      return;
    }
    setState(() => _saving = true);
    await widget.store.addProject(_projectName.text, _projectPath.text);
    if (!mounted) return;
    setState(() {
      _saving = false;
      if (widget.store.error == null) {
        _adding = false;
        _projectName.clear();
        _projectPath.clear();
      }
    });
  }

  Future<void> _renameProject(String id) async {
    if (_saving || _rename.text.trim().isEmpty) return;
    setState(() => _saving = true);
    await widget.store.renameProject(id, _rename.text);
    if (!mounted) return;
    setState(() {
      _saving = false;
      if (widget.store.error == null) _renaming = null;
    });
  }

  Widget _projects(BuildContext context) {
    final c = context.colors;
    final store = widget.store;
    return SettingsRow(
      label: 'Projects',
      description: 'Each project is a folder the agent can use as its working directory.',
      stack: true,
      trailing: Column(
        key: const ValueKey('agent-projects'),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final project in store.projects)
            Padding(
              key: ValueKey('agent-project-${project.id}'),
              padding: const EdgeInsets.only(bottom: 8),
              child: Row(
                children: [
                  Expanded(
                    child: _renaming == project.id
                        ? SettingsField(
                            fieldKey: const ValueKey(
                              'agent-project-rename-field',
                            ),
                            controller: _rename,
                            onSubmitted: (_) => _renameProject(project.id),
                          )
                        : Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                project.name,
                                style: tsRaw(14, 20, 400, c.textPrimary),
                              ),
                              Text(
                                project.path,
                                style: tsRaw(12, 16, 400, c.textSecondary),
                                softWrap: true,
                              ),
                            ],
                          ),
                  ),
                  const SizedBox(width: 8),
                  if (_renaming == project.id) ...[
                    TextBtn(
                      'Save',
                      key: const ValueKey('agent-project-rename-save'),
                      onTap: _saving ? null : () => _renameProject(project.id),
                    ),
                    TextBtn(
                      'Cancel',
                      onTap: () => setState(() => _renaming = null),
                    ),
                  ] else ...[
                    TextBtn(
                      'Rename',
                      key: ValueKey('agent-project-rename-${project.id}'),
                      onTap: () => setState(() {
                        _renaming = project.id;
                        _rename.text = project.name;
                      }),
                    ),
                    TextBtn(
                      'Remove',
                      key: ValueKey('agent-project-remove-${project.id}'),
                      color: c.inlineCodeText,
                      onTap: store.projects.length <= 1
                          ? null
                          : () => store.removeProject(project.id),
                    ),
                  ],
                ],
              ),
            ),
          Align(
            alignment: Alignment.centerLeft,
            child: TextBtn(
              'Add project',
              key: const ValueKey('agent-project-add'),
              onTap: () => setState(() => _adding = true),
            ),
          ),
          if (_adding) ...[
            const SizedBox(height: 8),
            SettingsField(
              fieldKey: const ValueKey('agent-project-name'),
              controller: _projectName,
              placeholder: 'Project name',
            ),
            const SizedBox(height: 8),
            Row(
              children: [
                Expanded(
                  child: SettingsField(
                    fieldKey: const ValueKey('agent-project-path'),
                    controller: _projectPath,
                    placeholder: 'Folder path',
                    mono: true,
                  ),
                ),
                if (FileDialogs.instance.folders) ...[
                  const SizedBox(width: 8),
                  TextBtn(
                    'Browse',
                    key: const ValueKey('agent-project-browse'),
                    onTap: _pickFolder,
                  ),
                ],
              ],
            ),
            const SizedBox(height: 8),
            Row(
              children: [
                SecondaryButton(
                  label: 'Add',
                  key: const ValueKey('agent-project-add-save'),
                  height: 32,
                  onTap: _saving ? null : _addProject,
                ),
                const SizedBox(width: 8),
                TextBtn('Cancel', onTap: () => setState(() => _adding = false)),
              ],
            ),
          ],
        ],
      ),
    );
  }
}
