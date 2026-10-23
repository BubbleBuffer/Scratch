// Settings. Desktop: a centred panel over a dimmed page, the pages listed
// on the left in groups, the page on the right. Mobile: a full-screen list
// of the pages in the same groups, each opening its own screen under a
// "‹ Settings" crumb.
//
// Pages: General, Reminders; Your vault: Sync and devices, Conflicts,
// Storage; AI: Agent, Connectors; About.

import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../core/file_dialogs.dart';
import '../../feedback/feedback_host.dart';
import '../../state/app_state.dart';
import '../../state/reminder_store.dart';
import '../../state/sync_store.dart';
import '../../shell/addons.dart';
import '../../ui/app_name.dart';
import '../../ui/ui.dart';
import '../ai/ai_settings.dart' show AgentHost, hubAgentPlatform;
import '../ai/ai_settings_body.dart';
import '../ai/ai_store.dart';
import '../mounts/mounts.dart';
import '../sync/conflicts.dart';
import '../sync/sync_settings.dart';
import 'connectors_body.dart';
import 'controls.dart';
import 'settings_kit.dart';
import 'settings_store.dart';

const appVersion = '0.1.0';

enum SettingsSection {
  general('General', Sx.general),
  reminders('Reminders', Sx.reminders),
  sync('Sync and devices', Sx.sync, group: 'Your vault'),
  conflicts('Conflicts', Sx.conflicts, group: 'Your vault'),
  storage('Storage', Sx.storage, group: 'Your vault'),
  agent('Agent', Sx.agent, group: 'AI'),
  connectors('Connectors', Sx.connectors, group: 'AI'),
  about('About', Sx.info);

  const SettingsSection(this.label, this.icon, {this.group});
  final String label;
  final Sx icon;

  /// The nav heading this page sits under, or null.
  final String? group;

  static List<SettingsSection> get visible => values;
}

Widget sectionIcon(SettingsSection s, Color color, double size) =>
    SIcon(s.icon, size: size, color: color);

Widget sectionBody(SettingsSection s) => switch (s) {
  SettingsSection.general => const PreferencesBody(),
  SettingsSection.reminders => const RemindersBody(),
  SettingsSection.sync => const _SyncAndDevicesBody(),
  SettingsSection.conflicts => const ConflictsBody(),
  SettingsSection.storage => const VaultBody(),
  SettingsSection.agent => const AiSettingsBody(),
  SettingsSection.connectors => const ConnectorsBody(),
  SettingsSection.about => const AboutBody(),
};

/// Sync, then (desktop) the devices paired with this computer's hub, then
/// add-on sections placed with them ("Run schedules on this device").
class _SyncAndDevicesBody extends StatelessWidget {
  const _SyncAndDevicesBody();

  @override
  Widget build(BuildContext context) {
    final extra = appAddonContext.settingsSections.all.where(
      (s) => s.location == 'addons' && (s.isVisible?.call(context) ?? true),
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SyncBody(),
        if (SyncStore.canHost) ...[
          const SizedBox(height: 24),
          const DevicesBody(),
        ],
        for (final s in extra)
          SSection(title: s.label, children: [s.build(context)]),
      ],
    );
  }
}

/// Opens settings at [section]. With none, a phone opens the list of
/// pages and a computer opens General.
Future<void> showSettings(BuildContext context, {SettingsSection? section}) {
  if (context.mobile) {
    if (section == null) {
      return Navigator.of(context).push(
        pageRoute(
          (_) => const SettingsPage(),
          motion: Motion.of(context),
          mobile: true,
          settings: const RouteSettings(name: '/settings'),
        ),
      );
    }
    return pushSectionPage(context, section);
  }
  return showPanel(
    context,
    (_) => SettingsModal(initial: section ?? SettingsSection.general),
    width: 1150,
    height: 715,
  );
}

/// Opens [section]: in the settings already open on a desktop, else as
/// [showSettings] does.
Future<void> openSettingsSection(
  BuildContext context,
  SettingsSection section,
) {
  final jump = context.dependOnInheritedWidgetOfExactType<_SettingsJump>();
  if (jump == null) return showSettings(context, section: section);
  jump.go(section);
  return Future.value();
}

/// Lets a section of the desktop modal open another ([openSettingsSection]).
class _SettingsJump extends InheritedWidget {
  const _SettingsJump({required this.go, required super.child});
  final ValueChanged<SettingsSection> go;

  @override
  bool updateShouldNotify(_SettingsJump old) => false;
}

// ---------------------------------------------------------------------------
// Frames

/// A desktop modal panel: centred, [width] x [height] at most (and 100 px
/// smaller than the window), over the overlay colour. A [shrink] panel is
/// as tall as its content up to that. Escape closes it.
Future<T?> showPanel<T>(BuildContext context, WidgetBuilder builder,
        {double width = 720, double height = 640, bool shrink = false}) =>
    Navigator.of(context).push(
        _PanelRoute<T>(builder: builder, width: width, height: height, shrink: shrink, barrier: context.colors.overlay, duration: Motion.of(context).fast));

class _PanelRoute<T> extends PopupRoute<T> {
  _PanelRoute({required this.builder, required this.width, required this.height, required this.shrink, required this.barrier, required this.duration});
  final WidgetBuilder builder;
  final double width, height;
  final bool shrink;
  final Color barrier;
  final Duration duration;

  @override
  Color? get barrierColor => barrier;
  @override
  bool get barrierDismissible => true;
  @override
  String? get barrierLabel => 'Close';
  @override
  Duration get transitionDuration => duration;

  @override
  Widget buildPage(BuildContext context, Animation<double> animation, Animation<double> secondaryAnimation) {
    final size = MediaQuery.sizeOf(context);
    final w = (size.width - 100).clamp(320.0, width);
    final h = (size.height - 100).clamp(320.0, height);
    return Center(
      child: CallbackShortcuts(
        bindings: {const SingleActivator(LogicalKeyboardKey.escape): () => Navigator.of(context).maybePop()},
        child: Focus(
          autofocus: true,
          child: ConstrainedBox(
            constraints: shrink ? BoxConstraints(maxWidth: w, minWidth: w, maxHeight: h) : BoxConstraints.tight(Size(w, h)),
            child: MenuSurface(child: builder(context)),
          ),
        ),
      ),
    );
  }

  @override
  Widget buildTransitions(
          BuildContext context, Animation<double> animation, Animation<double> secondaryAnimation, Widget child) =>
      FadeTransition(opacity: animation, child: child);
}

/// Mobile: a full-screen page with a back button and [title].
Future<void> pushSettingsPage(BuildContext context, String title, Widget body, {Key? key}) =>
    Navigator.of(context).push(pageRoute((_) => MobileSettingsScaffold(key: key, title: title, body: body), motion: Motion.of(context), mobile: true));

/// Mobile: one settings page under a "‹ Settings" crumb; the page shows
/// its own title.
Future<void> pushSectionPage(BuildContext context, SettingsSection s) =>
    Navigator.of(context).push(
      pageRoute(
        (_) => MobileSettingsScaffold(
          key: ValueKey('settings-page-${s.name}'),
          title: 'Settings',
          crumb: true,
          body: _SettingsJump(
            go: (next) => pushSectionPage(context, next),
            child: sectionBody(s),
          ),
        ),
        motion: Motion.of(context),
        mobile: true,
      ),
    );

class MobileSettingsScaffold extends StatelessWidget {
  const MobileSettingsScaffold({
    super.key,
    required this.title,
    required this.body,
    this.padded = true,
    this.crumb = false,
  });
  final String title;
  final Widget body;
  final bool padded;

  /// Shows [title] as a back crumb ("‹ Settings") instead of a heading.
  final bool crumb;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final mq = MediaQuery.of(context);
    return ColoredBox(
      color: c.bgPrimary,
      child: Padding(
        padding: EdgeInsets.only(top: mq.padding.top),
        child: Column(children: [
          SizedBox(
            height: Layout.topbarHeight,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 4),
              child: crumb
                  ? Row(children: [
                      Hover(
                        key: const ValueKey('back'),
                        onTap: () => Navigator.of(context).maybePop(),
                        lift: 0,
                        padding: const EdgeInsets.fromLTRB(6, 0, 12, 0),
                        child: Semantics(
                          button: true,
                          label: 'Back to $title',
                          child: SizedBox(
                            height: 44,
                            child: Row(mainAxisSize: MainAxisSize.min, children: [
                              SIcon(Sx.chevronLeft, size: 20, color: c.textSecondary),
                              const SizedBox(width: 2),
                              Text(title, style: tsRaw(15, 20, 600, c.textSecondary)),
                            ]),
                          ),
                        ),
                      ),
                    ])
                  : Row(children: [
                      IconBtn(Ico.chevronLeft,
                          key: const ValueKey('back'),
                          onTap: () => Navigator.of(context).maybePop(),
                          size: 44,
                          glyph: 24,
                          color: c.iconPrimary,
                          semanticLabel: 'Back'),
                      Expanded(child: Text(title, maxLines: 1, overflow: TextOverflow.ellipsis, style: tsDisplay(const TypeSpec(19, 24, 640), c.textPrimary))),
                    ]),
            ),
          ),
          Expanded(
            child: SingleChildScrollView(
              padding: EdgeInsets.fromLTRB(padded ? Layout.gutter : 0, 8, padded ? Layout.gutter : 0, mq.padding.bottom + 48),
              child: body,
            ),
          ),
        ]),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Desktop modal

class SettingsModal extends StatefulWidget {
  const SettingsModal({super.key, this.initial = SettingsSection.general});
  final SettingsSection initial;

  @override
  State<SettingsModal> createState() => _SettingsModalState();
}

class _SettingsModalState extends State<SettingsModal> {
  late SettingsSection _section = widget.initial;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final conflicts = context.watch<SyncStore>().conflicts.length;
    String? group;
    final nav = <Widget>[];
    for (final s in SettingsSection.visible) {
      if (s.group != group) {
        group = s.group;
        if (group != null) nav.add(_NavGroup(group));
      }
      nav.add(_NavItem(
        key: ValueKey('settings-nav-${s.name}'),
        section: s,
        selected: s == _section,
        badge: s == SettingsSection.conflicts && conflicts > 0 ? '$conflicts' : null,
        onTap: () => setState(() => _section = s),
      ));
      // About sits apart from the AI group.
      if (s == SettingsSection.connectors) {
        group = null;
        nav.add(const SizedBox(height: 12));
      }
    }
    return Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Container(
        width: 236,
        decoration: BoxDecoration(
          color: c.bgSecondary,
          border: Border(right: BorderSide(color: c.border)),
        ),
        padding: const EdgeInsets.fromLTRB(10, 18, 10, 18),
        child: SingleChildScrollView(
          child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(10, 0, 10, 10),
              child: Semantics(
                header: true,
                child: Text('Settings', style: tsDisplay(const TypeSpec(19, 24, 640), c.textPrimary)),
              ),
            ),
            ...nav,
          ]),
        ),
      ),
      Expanded(
        child: ColoredBox(
          color: c.bgPrimary,
          child: SingleChildScrollView(
            key: ValueKey('settings-body-${_section.name}'),
            padding: const EdgeInsets.fromLTRB(48, 36, 48, 64),
            child: Align(
              alignment: Alignment.topLeft,
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 720),
                child: _SettingsJump(
                  go: (s) => setState(() => _section = s),
                  child: KeyedSubtree(
                    key: ValueKey('settings-page-${_section.name}'),
                    child: sectionBody(_section),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    ]);
  }
}

class _NavGroup extends StatelessWidget {
  const _NavGroup(this.label);
  final String label;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(10, 16, 10, 4),
    child: Text(label, style: tsRaw(11, 16, 720, context.colors.textTertiary)),
  );
}

class _NavItem extends StatelessWidget {
  const _NavItem({super.key, required this.section, required this.selected, required this.onTap, this.badge});
  final SettingsSection section;
  final bool selected;
  final VoidCallback onTap;
  final String? badge;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.only(bottom: 1),
      child: Semantics(
        selected: selected,
        child: Hover(
          onTap: onTap,
          selected: selected,
          selectedColor: c.bgPrimary,
          lift: 0,
          radius: Radii.md,
          border: selected ? Border.all(color: c.border) : null,
          child: Container(
            constraints: const BoxConstraints(minHeight: 32),
            decoration: BoxDecoration(
              border: Border(
                left: BorderSide(color: selected ? c.accent : const Color(0x00000000), width: 3),
              ),
            ),
            padding: const EdgeInsets.fromLTRB(7, 6, 10, 6),
            child: Row(children: [
              sectionIcon(section, selected ? c.iconPrimary : c.iconSecondary, 16),
              const SizedBox(width: 10),
              Expanded(
                child: Text(section.label,
                    style: tsRaw(13.5, 18, selected ? 620 : 500, selected ? c.textPrimary : c.textSecondary)),
              ),
              if (badge != null) _Badge(badge!),
            ]),
          ),
        ),
      ),
    );
  }
}

class _Badge extends StatelessWidget {
  const _Badge(this.text);
  final String text;
  @override
  Widget build(BuildContext context) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 6),
        height: 18,
        alignment: Alignment.center,
        decoration: BoxDecoration(color: context.colors.attnBg, borderRadius: Radii.pill),
        child: Text(text, style: tsRaw(11, 14, 650, context.colors.textPrimary)),
      );
}

// ---------------------------------------------------------------------------
// Mobile list

class SettingsPage extends StatelessWidget {
  const SettingsPage({super.key});

  String? _value(BuildContext context, SettingsSection s) {
    final sync = context.watch<SyncStore>();
    final settings = AppSettings.of(context);
    return switch (s) {
      SettingsSection.general => settings.theme == ThemePref.system ? 'System theme' : settings.theme.label,
      SettingsSection.reminders => settings.reminders.enabled ? 'On' : 'Off',
      SettingsSection.sync => sync.connected ? 'Synced' : (sync.hosting ? 'Hub on' : 'Off'),
      SettingsSection.conflicts => sync.conflicts.isEmpty ? null : '${sync.conflicts.length}',
      SettingsSection.agent => _agentValue(context),
      _ => null,
    };
  }

  String _agentValue(BuildContext context) {
    final ai = AiStore.of(context.app);
    if (hubAgentPlatform) return 'Hub';
    if (!ai.settings.agentEnabled) return 'Off';
    return ai.runsOn == AgentHost.hub ? 'Hub' : 'This computer';
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    String? group;
    final rows = <Widget>[];
    for (final s in SettingsSection.visible) {
      if (s.group != group) {
        group = s.group;
        if (group != null) {
          rows.add(Padding(
            padding: const EdgeInsets.fromLTRB(Layout.gutter, 20, Layout.gutter, 6),
            child: Text(group, style: tsRaw(12, 16, 720, c.textTertiary)),
          ));
        }
      }
      if (s == SettingsSection.about) rows.add(const SizedBox(height: 20));
      rows.add(Hover(
        key: ValueKey('settings-nav-${s.name}'),
        onTap: () => pushSectionPage(context, s),
        radius: 0,
        lift: 0,
        padding: const EdgeInsets.symmetric(horizontal: Layout.gutter),
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 48),
          child: Row(children: [
            sectionIcon(s, c.iconSecondary, 20),
            const SizedBox(width: 14),
            Expanded(child: Text(s.label, style: tsRaw(15, 20, 500, c.textPrimary))),
            if (_value(context, s) case final v?) ...[
              Text(v, style: tsRaw(14, 20, 400, c.textTertiary)),
              const SizedBox(width: 6),
            ],
            SIcon(Sx.chevronRight, size: 16, color: c.iconTertiary),
          ]),
        ),
      ));
    }
    return MobileSettingsScaffold(
      title: '',
      padded: false,
      body: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(Layout.gutter, 0, Layout.gutter, 4),
          child: Semantics(
            header: true,
            child: Text('Settings', style: tsDisplay(const TypeSpec(24, 28, 680), c.textPrimary)),
          ),
        ),
        ...rows,
      ]),
    );
  }
}

// ---------------------------------------------------------------------------
// Sections without sync

class PreferencesBody extends StatelessWidget {
  const PreferencesBody({super.key});

  @override
  Widget build(BuildContext context) {
    final s = AppSettings.of(context);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SettingsHeading('Preferences', primary: true),
      SettingsRow(
        label: 'Appearance',
        description: 'Customize how $appName looks on this device.',
        trailing: SelectButton<ThemePref>(
          key: const ValueKey('theme-select'),
          title: 'Appearance',
          value: s.theme,
          options: ThemePref.values,
          labelOf: (t) => t.label,
          onChanged: (t) => s.update((s) => s.theme = t),
        ),
      ),
      const SettingsHeading('Feedback', top: 24),
      SettingsRow(
        label: 'Send feedback',
        description: 'Write what happened. A screenshot and the app’s diagnostics go with it.',
        trailing: TextBtn(
          'Open',
          key: const ValueKey('send-feedback'),
          onTap: () => FeedbackHost.of(context).open(context),
        ),
      ),
      if (defaultTargetPlatform == TargetPlatform.android ||
          defaultTargetPlatform == TargetPlatform.iOS)
        SettingsRow(
          label: 'Shake to send feedback',
          description: 'Shake the phone to open the feedback sheet.',
          trailing: Toggle(
            key: const ValueKey('shake-to-send'),
            value: s.shakeToSend,
            onChanged: (on) => s.update((s) => s.shakeToSend = on),
          ),
        ),
    ]);
  }
}

class RemindersBody extends StatelessWidget {
  const RemindersBody({super.key});

  static String _time(int minutes) =>
      '${(minutes ~/ 60).toString().padLeft(2, '0')}:${(minutes % 60).toString().padLeft(2, '0')}';

  @override
  Widget build(BuildContext context) {
    final s = AppSettings.of(context);
    final r = s.reminders;
    final store = context.watch<ReminderStore>();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SettingsHeading('Reminders', primary: true),
        SettingsRow(
          label: 'Reminders',
          trailing: Toggle(
            key: const ValueKey('reminders-enabled'),
            value: r.enabled,
            onChanged: (enabled) =>
                s.update((s) => s.reminders = r.copyWith(enabled: enabled)),
          ),
        ),
        SettingsRow(
          label: 'Remind me before events',
          trailing: SelectButton<int>(
            key: const ValueKey('reminders-lead'),
            title: 'Remind me before events',
            value: r.leadMinutes,
            options: const [0, 5, 10, 30],
            labelOf: (minutes) => minutes == 0 ? 'At start' : '$minutes min',
            onChanged: r.enabled
                ? (lead) => s.update(
                    (s) => s.reminders = r.copyWith(leadMinutes: lead),
                  )
                : null,
          ),
        ),
        SettingsRow(
          label: 'Morning summary at',
          trailing: SelectButton<int>(
            key: const ValueKey('reminders-morning'),
            title: 'Morning summary at',
            value: r.morning,
            options: [
              for (var minutes = 300; minutes <= 720; minutes += 30) minutes,
            ],
            labelOf: _time,
            onChanged: r.enabled
                ? (morning) => s.update(
                    (s) => s.reminders = r.copyWith(morning: morning),
                  )
                : null,
          ),
        ),
        if (defaultTargetPlatform == TargetPlatform.android &&
            store.permissionGranted == false)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(
              'Notifications are off for $appName in Android settings.',
              key: const ValueKey('reminders-permission-note'),
              style: tsRaw(13, 18, 400, context.colors.textSecondary),
            ),
          ),
      ],
    );
  }
}

class VaultBody extends StatefulWidget {
  const VaultBody({super.key});

  @override
  State<VaultBody> createState() => _VaultBodyState();
}

class _VaultBodyState extends State<VaultBody> {
  final _path = TextEditingController();
  String? _error;

  @override
  void dispose() {
    _path.dispose();
    super.dispose();
  }

  /// Picks the folder in the system dialog and uses it.
  Future<void> _choose(AppSettings s) async {
    final dir = await FileDialogs.instance.pickFolder(title: 'Open a vault folder');
    if (dir == null || !mounted) return;
    _path.text = dir;
    _use(s);
  }

  void _use(AppSettings s) {
    final p = _path.text.trim();
    if (p.isEmpty) return;
    final d = Directory(p);
    if (d.existsSync() && d.listSync().isNotEmpty && !Directory('$p${Platform.pathSeparator}.vault').existsSync()) {
      setState(() => _error = 'That folder has other files in it. Choose an empty folder or an existing vault.');
      return;
    }
    setState(() => _error = null);
    s.update((s) => s.vaultPath = p);
    _path.clear();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final s = AppSettings.of(context);
    final root = context.app.vault.root;
    final pending = s.vaultPath != null && s.vaultPath != root ? s.vaultPath : null;
    final desktop = !context.mobile;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SettingsHeading('Vault', primary: true),
      SettingsRow(
        label: 'Location',
        description: 'The folder that holds your pages, as Markdown files and their history.',
        stack: true,
        trailing: CopyableText(root, textKey: const ValueKey('vault-path')),
      ),
      if (pending != null)
        Padding(
          padding: const EdgeInsets.only(top: 4, bottom: 8),
          child: Text('$appName opens $pending the next time it starts.',
              key: const ValueKey('vault-pending'), style: tsRaw(13, 18, 400, c.textSecondary)),
        ),
      if (desktop) ...[
        const SizedBox(height: 8),
        SettingsRow(
          label: 'Open another folder',
          description: 'An empty folder starts a new vault; a vault folder opens it. Takes effect after a restart.',
          stack: true,
          trailing: Row(children: [
            Expanded(
              child: SettingsField(
                fieldKey: const ValueKey('vault-path-field'),
                controller: _path,
                placeholder: Platform.isWindows ? r'D:\Notes' : '/home/me/Notes',
                onSubmitted: (_) => _use(s),
              ),
            ),
            if (FileDialogs.instance.folders) ...[
              const SizedBox(width: 8),
              OutlineButton(key: const ValueKey('vault-choose'), label: 'Choose…', onTap: () => _choose(s)),
            ],
            const SizedBox(width: 8),
            OutlineButton(key: const ValueKey('vault-use'), label: 'Use folder', onTap: () => _use(s)),
          ]),
        ),
        if (_error != null) Text(_error!, style: tsRaw(13, 18, 400, c.inlineCodeText)),
        if (s.vaultPath != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Align(
              alignment: Alignment.centerLeft,
              child: TextBtn('Use the default location', onTap: () => s.update((s) => s.vaultPath = null), color: c.textSecondary),
            ),
          ),
        const MountsSection(),
      ],
    ]);
  }
}

class AboutBody extends StatelessWidget {
  const AboutBody({super.key});

  @override
  Widget build(BuildContext context) {
    final vault = context.app.vault;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SettingsHeading('About', primary: true),
      const SettingsRow(label: 'Version', description: '$appName $appVersion'),
      SettingsRow(
        label: 'Device ID',
        description: 'Identifies this device to the hub and in conflicts.',
        stack: true,
        trailing: CopyableText(vault.deviceId, textKey: const ValueKey('device-id')),
      ),
      SettingsRow(
        label: 'Add-ons',
        description: 'Manage the features installed on this device.',
        trailing: TextBtn('Manage', key: const ValueKey('manage-addons'),
          onTap: () => context.mobile
            ? pushSettingsPage(context, 'Add-ons', const AddonsBody())
            : showPanel(context, (_) => const Padding(
                padding: EdgeInsets.all(24), child: AddonsBody()), shrink: true)),
      ),
    ]);
  }
}

class AddonsBody extends StatelessWidget {
  const AddonsBody({super.key});
  @override
  Widget build(BuildContext context) {
    final settings = AppSettings.of(context);
    return Column(mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SettingsHeading('Add-ons', primary: true),
      const Text('Changes take effect when you restart $appName.'),
      for (final entry in addonList)
        SettingsRow(label: entry.label,
          description: entry.optional ? 'Optional' : 'Required',
          trailing: entry.optional
            ? Toggle(key: ValueKey('addon-${entry.addon.id}'),
                value: !settings.disabledAddons.contains(entry.addon.id),
                onChanged: (enabled) => settings.update((s) {
                  if (enabled) { s.disabledAddons.remove(entry.addon.id); }
                  else { s.disabledAddons.add(entry.addon.id); }
                }))
            : const Text('Enabled')),
    ]);
  }
}
