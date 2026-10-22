import '../../services/addon_services.dart';
// Settings > AI > Agent (desktop): whether the in-app agent runs, where
// it runs (this computer, or the hub this vault syncs with when that is an
// https:// hub with a device token), its default model (listed from the
// default host, even while the tab is on the other one), and for this
// computer the Claude Code extension and projects. The MCP servers
// ([McpServersSettings]) follow the tool sets. The default runner and
// model are vault settings (synced); "On this device" overrides them here
// only (ai_settings.json, like the rest of this section). Each runner's
// tool set is a vault setting too, read by that runner when its agent starts.
// Permission preferences remain dormant while the app always uses Bypass.

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

import '../../ui/toast.dart';
import '../../ui/ui.dart';
import '../../core/file_dialogs.dart';
import '../../core/tool_sets.dart';
import 'agent_mcp_settings.dart';
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
  final _customModel = TextEditingController();
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
      _customModel,
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

  Future<void> _write(Future<void> Function() f) async {
    try {
      await f();
    } catch (e) {
      if (mounted) {
        showErrorToast(context, 'Could not change this setting. Try again.');
      }
    }
    widget.onChanged();
    if (mounted) setState(() {});
  }

  static String _from(AgentDefaultSource? source) => switch (source) {
    AgentDefaultSource.device => 'set on this device',
    AgentDefaultSource.vault => 'the vault setting',
    _ => 'the built-in default',
  };

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
    final hub = store.hubLink;
    final onHub = store.host == AgentHost.hub;
    final resolved = store.resolvedHost;
    final vaultHost = store.vaultDefaults.host ?? builtInAgentHost;
    final hostOverride = s.agentHostOverride;
    // Default model: the default host's list, not the tab's.
    final def = store.defaultHost;
    final defaultOnHub = def == AgentHost.hub;
    final (:ids, :unavailable) = store.defaultHostModels();
    final models = ids ?? const <String>[];
    final vaultModel = store.vaultDefaults.models[def];
    final modelOverride = s.agentModelOverrides[def];
    final effective = store.resolvedModel(def);
    final vaultCurrent = vaultModel ?? (models.isEmpty ? '' : models.first);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SettingsRow(
          label: 'Use the agent',
          description: 'Chat with the agent in the Agent section. It reads and edits this vault on this device.',
          trailing: Toggle(
            key: const ValueKey('agent-enabled-toggle'),
            value: s.agentEnabled,
            onChanged: (v) => _set((s) => s.agentEnabled = v),
          ),
        ),
        if (s.agentEnabled) ...[
          SettingsRow(
            label: 'Agent runs on',
            description: vaultHost == AgentHost.hub
                ? 'Default for new chats on every device of this vault: the hub this vault syncs with. Each chat can pick before its first message.'
                : 'Default for new chats on every device of this vault: the device in use. Each chat can pick before its first message.',
            trailing: SelectButton<AgentHost>(
              key: const ValueKey('agent-host'),
              title: 'Agent runs on',
              value: vaultHost,
              options: AgentHost.values,
              labelOf: (h) => h.label,
              onChanged: (h) => _write(() => store.setVaultDefaultHost(h)),
            ),
          ),
          SettingsRow(
            label: 'On this device',
            description: hostOverride == null
                ? 'Follows the vault setting.'
                : 'Overrides the vault setting on this device only.',
            trailing: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                SelectButton<AgentHost?>(
                  key: const ValueKey('agent-host-override'),
                  title: 'On this device',
                  value: hostOverride,
                  options: [
                    AgentHost.local,
                    if (hub != null || hostOverride == AgentHost.hub)
                      AgentHost.hub,
                  ],
                  labelOf: (h) => h?.label ?? 'Vault setting',
                  onChanged: (h) => _set((s) => s.agentHostOverride = h),
                ),
                if (hostOverride != null) ...[
                  const SizedBox(width: 8),
                  SecondaryButton(
                    key: const ValueKey('agent-host-override-clear'),
                    label: 'Clear',
                    onTap: () => _set((s) => s.agentHostOverride = null),
                  ),
                ],
              ],
            ),
          ),
          if (resolved.fellBack)
            Text(
              "${resolved.wanted.label} (${_from(resolved.source)}) isn't reachable from this device, so new chats run on ${resolved.host.label}."
              "${hub == null ? ' The hub becomes available when this vault syncs with an https:// hub.' : ''}",
              key: const ValueKey('agent-host-fallback'),
              style: tsRaw(12, 16, 400, c.textSecondary),
            ),
        ],
        if (s.agentEnabled && onHub && store.openError != null)
          Text(
            store.openError!,
            key: const ValueKey('agent-settings-error'),
            style: tsRaw(12, 16, 400, c.inlineCodeText),
          ),
        if (s.agentEnabled && store.hostDescription != null)
          Text(
            store.hostDescription!,
            key: const ValueKey('agent-host-info'),
            style: tsRaw(12, 16, 400, c.textSecondary),
          ),
        if (s.agentEnabled) ...[
          _models(context),
          if (models.isNotEmpty || unavailable) ...[
            SettingsRow(
              label: 'Default model',
              description: defaultOnHub
                  ? "For new chats on the hub, on every device of this vault; the hub's first model if it doesn't offer it. Each chat can pick another."
                  : "For new chats on this computer and AI tasks, on every device of this vault; this computer's first model if it doesn't offer it. Each chat can pick another.",
              trailing: models.isNotEmpty
                  ? SelectButton<String>(
                      key: const ValueKey('agent-default-model'),
                      title: 'Default model',
                      value: models.contains(vaultCurrent)
                          ? vaultCurrent
                          : models.first,
                      options: models,
                      labelOf: store.modelLabel,
                      onChanged: (m) =>
                          _write(() => store.setVaultDefaultModel(def, m)),
                    )
                  : Text(
                      vaultModel == null
                          ? 'Not set'
                          : store.modelLabel(vaultModel),
                      key: const ValueKey('agent-default-model-stored'),
                      style: tsRaw(13, 18, 400, c.textSecondary),
                    ),
            ),
            SettingsRow(
              label: 'Model on this device',
              description: modelOverride == null
                  ? 'Follows the vault setting.'
                  : 'Overrides the vault setting on this device only.',
              trailing: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  if (models.isNotEmpty || modelOverride != null)
                    SelectButton<String?>(
                      key: const ValueKey('agent-model-override'),
                      title: 'Model on this device',
                      value: modelOverride,
                      options: [
                        ...models,
                        if (modelOverride != null &&
                            !models.contains(modelOverride))
                          modelOverride,
                      ],
                      labelOf: (m) =>
                          m == null ? 'Vault setting' : store.modelLabel(m),
                      onChanged: (m) {
                        store.setModelOverride(def, m);
                        widget.onChanged();
                      },
                    ),
                  if (modelOverride != null) ...[
                    const SizedBox(width: 8),
                    SecondaryButton(
                      key: const ValueKey('agent-model-override-clear'),
                      label: 'Clear',
                      onTap: () {
                        store.setModelOverride(def, null);
                        widget.onChanged();
                      },
                    ),
                  ],
                ],
              ),
            ),
          ],
          if (models.isEmpty && unavailable)
            Text(
              defaultOnHub
                  ? "Can't list the hub's models right now."
                  : "Can't list this computer's models right now.",
              key: const ValueKey('agent-default-model-unavailable'),
              style: tsRaw(12, 16, 400, c.textSecondary),
            ),
          if (effective.model != null)
            Text(
              'New chats here use ${store.modelLabel(effective.model!)} (${_from(effective.source)}).',
              key: const ValueKey('agent-default-model-effective'),
              style: tsRaw(12, 16, 400, c.textSecondary),
            ),
          for (final h in AgentHost.values) _toolSet(h),
          McpServersSettings(store: store),
        ],
        if (s.agentEnabled && !onHub) ...[
          field(
            'Claude Code: bun program',
            'Optional override. Empty uses the detected bun program.',
            _bun,
            'agent-bun-path',
            store.claudeDetection?.bunPath ?? 'bun',
            (s, v) => s.agentBunPath = v,
          ),
          field(
            'Claude Code: extension folder',
            "Optional override. Empty uses the detected Claude Code extension.",
            _dir,
            'agent-claude-code-dir',
            store.claudeDetection?.claudeCodeDir ??
                'BubblePaw/extensions/claude-code',
            (s, v) => s.agentClaudeCodeDir = v,
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
          if (s.desktopTools && (store.hostInfo?.projects ?? false))
            _projects(context),
          if (store.error != null)
            Text(
              store.error!,
              key: const ValueKey('agent-project-error'),
              style: tsRaw(12, 16, 400, c.inlineCodeText),
            ),
          if (store.openError != null)
            Text(
              store.openError!,
              key: const ValueKey('agent-settings-error'),
              style: tsRaw(12, 16, 400, c.inlineCodeText),
            ),
        ],
      ],
    );
  }

  /// [runner]'s tool set, synced in the vault. The conversation tools come
  /// with every set; computer access only with Everything.
  Widget _toolSet(AgentHost runner) {
    final store = widget.store;
    final set = store.vaultDefaults.toolSetFor(runner);
    final onHub = runner == AgentHost.hub;
    return SettingsRow(
      label: onHub ? 'Tools on the hub' : 'Tools on this computer',
      description:
          '${toolSetDescription(set)} '
          '${onHub ? 'Applies after the hub restarts.' : 'Applies to new chats; other computers pick it up when their agent next starts.'}',
      trailing: SelectButton<String>(
        key: ValueKey('agent-tool-set-${runner.runner}'),
        title: onHub ? 'Tools on the hub' : 'Tools on this computer',
        value: set,
        options: toolSets,
        labelOf: toolSetLabel,
        onChanged: (v) => _write(() => store.setVaultToolSet(runner, v)),
      ),
    );
  }

  void _addModel() {
    final id = _customModel.text.trim();
    final separator = id.indexOf('/');
    if (separator <= 0 ||
        separator == id.length - 1 ||
        id.contains(RegExp(r'\s'))) {
      return;
    }
    _set((s) {
      if (!s.agentCustomModels.contains(id)) s.agentCustomModels.add(id);
      s.agentHiddenModels.remove(id);
    });
    _customModel.clear();
  }

  Widget _models(BuildContext context) {
    final store = widget.store;
    final settings = store.settings;
    String group(String id) => switch (id.split('/').first) {
      'codex' => 'Codex',
      'claude-code' => 'Claude',
      _ => 'Custom',
    };
    return SettingsRow(
      label: 'Models',
      description: 'Choose which models appear in the picker on this device.',
      stack: true,
      trailing: Column(
        key: const ValueKey('agent-models'),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final provider in ['Codex', 'Claude', 'Custom'])
            if (store.allModelIds.any((id) => group(id) == provider)) ...[
              Padding(
                padding: const EdgeInsets.only(top: 8, bottom: 4),
                child: Text(
                  provider,
                  style: ts(context.type.uiSmall, context.colors.textSecondary),
                ),
              ),
              for (final id in store.allModelIds.where(
                (id) => group(id) == provider,
              ))
                Padding(
                  padding: const EdgeInsets.only(bottom: 6),
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(
                          store.modelLabel(id),
                          style: ts(
                            context.type.uiSmall,
                            context.colors.textPrimary,
                          ),
                        ),
                      ),
                      if (settings.agentCustomModels.contains(id))
                        TextBtn(
                          'Remove',
                          key: ValueKey('agent-model-remove-$id'),
                          onTap: () => _set((s) {
                            s.agentCustomModels.remove(id);
                            s.agentHiddenModels.remove(id);
                          }),
                        ),
                      const SizedBox(width: 8),
                      Toggle(
                        key: ValueKey('agent-model-visible-$id'),
                        value: !settings.agentHiddenModels.contains(id),
                        onChanged: (shown) => _set((s) {
                          if (shown) {
                            s.agentHiddenModels.remove(id);
                          } else {
                            s.agentHiddenModels.add(id);
                          }
                        }),
                      ),
                    ],
                  ),
                ),
            ],
          const SizedBox(height: 8),
          Row(
            children: [
              Expanded(
                child: SettingsField(
                  fieldKey: const ValueKey('agent-model-add-field'),
                  controller: _customModel,
                  placeholder: 'Add model id: provider/model',
                  mono: true,
                  onSubmitted: (_) => _addModel(),
                ),
              ),
              const SizedBox(width: 8),
              TextBtn(
                'Add',
                key: const ValueKey('agent-model-add'),
                onTap: _addModel,
              ),
            ],
          ),
        ],
      ),
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
