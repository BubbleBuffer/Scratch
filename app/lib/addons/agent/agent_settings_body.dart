import '../../services/addon_services.dart';
// Settings > Agent (desktop): "Use the agent", then "Where it runs" (the
// vault's synced runner, [RunAiOnSetting]) with that runner's model, "What
// it may do" (its tool set), and, when it runs on this computer, "On this
// computer" (computer access and projects, which need Everything) and
// Advanced > Claude Code provider. MCP servers are on Settings >
// Connectors; one line links there. The runner, models and tool sets are
// vault settings (synced); the rest is this computer's (ai_settings.json).
// If the chosen runner isn't reachable here, chats fall back to the other
// one by themselves.

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import '../../core/file_dialogs.dart';
import 'agent_runner_settings.dart';
import 'agent_store.dart';
import 'host_relay_status.dart' show showMcpServers;

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
    final chosen = chosenRunner(store);
    final onHub = chosen == AgentHost.hub;
    final resolved = store.resolvedHost;
    final everything =
        store.vaultDefaults.toolSetFor(AgentHost.local) == 'everything';
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SSection(
          first: true,
          children: [
            SRow(
              label: 'Use the agent',
              desc: 'Show the Agent chat and Ask AI.',
              control: SSwitch(
                key: const ValueKey('agent-enabled-toggle'),
                label: 'Use the agent',
                value: s.agentEnabled,
                onChanged: (v) => _set((s) => s.agentEnabled = v),
              ),
            ),
          ],
        ),
        if (s.agentEnabled) ...[
          SSection(
            title: 'Where it runs',
            intro: 'One choice for all your devices.',
            children: [
              RunAiOnSetting(store: store, onChanged: widget.onChanged),
              if (resolved.fellBack && resolved.wanted == AgentHost.hub)
                SNotice(
                  store.hubLink == null
                      ? 'Chats run on this computer until this vault syncs with an https:// hub.'
                      : 'Chats run on this computer for now.',
                  kind: SNoticeKind.warn,
                  textKey: const ValueKey('agent-host-fallback'),
                ),
              if (onHub)
                HubModelSetting(store: store, onChanged: widget.onChanged)
              else
                ComputerModelSetting(store: store, onChanged: widget.onChanged),
            ],
          ),
          SSection(
            title: 'What it may do',
            intro: onHub
                ? 'Applies to the agent on your hub.'
                : 'Applies to the agent on this computer.',
            children: [
              RunnerToolSetSetting(
                store: store,
                runner: onHub ? AgentHost.hub : AgentHost.local,
                onChanged: widget.onChanged,
              ),
            ],
          ),
          if (!onHub) ...[
            SSection(
              title: 'On this computer',
              intro: 'Extra reach for the agent when it runs here.',
              children: [
                SRow(
                  label: 'Computer access',
                  desc:
                      'Lets the agent open project folders and run commands. **Commands are not sandboxed:** they can do anything you can.',
                  help: 'Turn off to keep the agent inside your notes.',
                  disabled: !everything,
                  reason: everything
                      ? null
                      : 'Needs “Everything” above. With Read or Write this switch does nothing.',
                  control: SSwitch(
                    key: const ValueKey('agent-desktop-tools'),
                    label: 'Computer access',
                    value: s.desktopTools,
                    onChanged: everything
                        ? (v) => _set((s) => s.desktopTools = v)
                        : null,
                  ),
                ),
                if (store.host == AgentHost.local &&
                    (store.hostInfo?.projects ?? false))
                  _projects(context, everything && s.desktopTools, everything),
                SLinkLine(
                  key: const ValueKey('agent-mcp-link'),
                  before: 'MCP servers are on ',
                  link: 'Connectors',
                  after: '.',
                  onTap: () => showMcpServers(context, open: false),
                ),
              ],
            ),
            if (store.error != null)
              SNotice(
                store.error!,
                kind: SNoticeKind.bad,
                textKey: const ValueKey('agent-project-error'),
              ),
            if (store.host == AgentHost.local && store.openError != null)
              SNotice(
                store.openError!,
                kind: SNoticeKind.bad,
                textKey: const ValueKey('agent-settings-error'),
              ),
            SAdvanced(
              title: 'Advanced',
              summary: 'Claude Code provider',
              headerKey: const ValueKey('agent-advanced'),
              initiallyOpen:
                  s.agentBunPath.isNotEmpty || s.agentClaudeCodeDir.isNotEmpty,
              children: [
                Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: Text.rich(
                    sRich(
                      'The agent can use **Claude Code** as a model provider, with your Claude Code sign-in. It needs the bun program and BubblePaw’s Claude Code extension. Scratch finds both on its own; leave these empty unless Claude Code is missing from Model.',
                      sDesc(context),
                    ),
                  ),
                ),
                _pathRow(
                  label: 'bun program',
                  desc: 'Path to bun.exe.',
                  help: 'Only if bun is installed somewhere Scratch doesn’t look.',
                  extra:
                      'A wrong path hides Claude Code entirely. Clear the field to go back to automatic.',
                  ctl: _bun,
                  key: 'agent-bun-path',
                  found: store.claudeDetection?.bunPath,
                  fallback: 'bun',
                  save: (s, v) => s.agentBunPath = v,
                ),
                _pathRow(
                  label: 'Claude Code extension folder',
                  desc: 'The folder that holds the extension’s src/main.ts.',
                  help:
                      'Only if you keep BubblePaw’s extension in your own checkout.',
                  ctl: _dir,
                  key: 'agent-claude-code-dir',
                  found: store.claudeDetection?.claudeCodeDir,
                  fallback: 'BubblePaw/extensions/claude-code',
                  save: (s, v) => s.agentClaudeCodeDir = v,
                ),
              ],
            ),
          ],
        ],
      ],
    );
  }

  Widget _pathRow({
    required String label,
    required String desc,
    required String help,
    String? extra,
    required TextEditingController ctl,
    required String key,
    required String? found,
    required String fallback,
    required void Function(AiSettings, String) save,
  }) => SRow(
    label: label,
    desc: desc,
    help: help,
    extra: extra,
    stack: true,
    control: SizedBox(
      width: double.infinity,
      child: STextField(
        fieldKey: ValueKey(key),
        controller: ctl,
        mono: true,
        placeholder: found == null ? fallback : 'Found: $found',
        onChanged: (v) => _set((s) => save(s, v.trim())),
      ),
    ),
  );

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

  Future<void> _removeProject(AgentProject project) async {
    final ok = await showConfirm(
      context,
      title: 'Remove “${project.name}”?',
      message:
          'The agent can no longer work in ${project.path}. The folder and its files stay as they are.',
      confirmLabel: 'Remove project',
      danger: true,
    );
    if (ok && mounted) await widget.store.removeProject(project.id);
  }

  Widget _projects(BuildContext context, bool on, bool everything) {
    final store = widget.store;
    final only = store.projects.length <= 1;
    final list = Column(
      key: const ValueKey('agent-projects'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SList(
          foot: _adding
              ? null
              : SListFoot(
                  children: [
                    SButton(
                      'Add project',
                      key: const ValueKey('agent-project-add'),
                      icon: Sx.plus,
                      onTap: () => setState(() => _adding = true),
                    ),
                    if (only)
                      Text(
                        'Only ${store.projects.firstOrNull?.name ?? 'one folder'} so far. Add a folder to let the agent work there.',
                        style: sHint(context),
                      ),
                  ],
                ),
          children: [
            for (final project in store.projects)
              KeyedSubtree(
                key: ValueKey('agent-project-${project.id}'),
                child: _renaming == project.id
                    ? Padding(
                        padding: const EdgeInsets.fromLTRB(12, 8, 8, 8),
                        child: Row(
                          children: [
                            Expanded(
                              child: STextField(
                                fieldKey: const ValueKey(
                                  'agent-project-rename-field',
                                ),
                                controller: _rename,
                                autofocus: true,
                                onSubmitted: (_) => _renameProject(project.id),
                              ),
                            ),
                            const SizedBox(width: 8),
                            SButton(
                              'Cancel',
                              kind: SButtonKind.quiet,
                              onTap: () => setState(() => _renaming = null),
                            ),
                            SButton(
                              'Save',
                              key: const ValueKey('agent-project-rename-save'),
                              kind: SButtonKind.primary,
                              onTap: _saving
                                  ? null
                                  : () => _renameProject(project.id),
                            ),
                          ],
                        ),
                      )
                    : SItem(
                        icon: Sx.folder,
                        name: project.name,
                        sub: project.path,
                        subMono: true,
                        trailing: [
                          SIconButton(
                            Sx.pencil,
                            key: ValueKey('agent-project-rename-${project.id}'),
                            label: 'Rename ${project.name}',
                            onTap: () => setState(() {
                              _renaming = project.id;
                              _rename.text = project.name;
                            }),
                          ),
                          if (!only)
                            SIconButton(
                              Sx.trash,
                              key: ValueKey(
                                'agent-project-remove-${project.id}',
                              ),
                              label: 'Remove ${project.name}',
                              danger: true,
                              onTap: () => _removeProject(project),
                            ),
                        ],
                      ),
              ),
          ],
        ),
        if (_adding)
          SEditor(
            title: 'Add a project',
            actions: [
              SButton(
                'Cancel',
                kind: SButtonKind.quiet,
                onTap: () => setState(() => _adding = false),
              ),
              SButton(
                'Add project',
                key: const ValueKey('agent-project-add-save'),
                kind: SButtonKind.primary,
                onTap: _saving ? null : _addProject,
              ),
            ],
            children: [
              SField(
                label: 'Folder',
                child: Row(
                  children: [
                    Expanded(
                      child: STextField(
                        fieldKey: const ValueKey('agent-project-path'),
                        controller: _projectPath,
                        placeholder: 'Folder path',
                        mono: true,
                      ),
                    ),
                    if (FileDialogs.instance.folders) ...[
                      const SizedBox(width: 8),
                      SButton(
                        'Browse…',
                        key: const ValueKey('agent-project-browse'),
                        icon: Sx.folder,
                        onTap: _pickFolder,
                      ),
                    ],
                  ],
                ),
              ),
              SField(
                label: 'Name',
                hint: 'Filled in from the folder name when you browse.',
                child: STextField(
                  fieldKey: const ValueKey('agent-project-name'),
                  controller: _projectName,
                  placeholder: 'Project name',
                ),
              ),
            ],
          ),
      ],
    );
    return SRow(
      label: 'Projects',
      desc:
          'Folders the agent can work in. In a chat, the agent starts in one of them and runs its commands there.',
      help:
          'Add a folder when you want the agent to work on its files, such as a code repository or a writing project.',
      stack: true,
      disabled: !on,
      reason: !everything
          ? 'Needs “Everything” above.'
          : !on
          ? 'Turn on Computer access to use projects.'
          : null,
      below: on ? list : null,
    );
  }
}
