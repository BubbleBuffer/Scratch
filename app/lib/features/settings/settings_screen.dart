// Settings. Desktop: a centred panel over a dimmed page, the pages listed
// on the left in groups, the page on the right. Mobile: a full-screen list
// of the pages in the same groups, each opening its own screen under a
// "‹ Settings" crumb.
//
// Pages: General, Reminders; Your vault: Sync and devices, Conflicts,
// Storage, Shared links (only while signed in to a hub); AI: Agent,
// Connectors; About (with Updates on Windows).

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../core/file_dialogs.dart';
import '../../feedback/feedback_host.dart';
import '../../state/app_state.dart';
import '../../state/reminder_store.dart';
import '../../state/sync_store.dart';
import '../../shell/addons.dart' show appAddonContext;
import '../../ui/app_name.dart';
import '../../ui/ui.dart';
import '../ai/ai_settings.dart' show AgentHost, hubAgentPlatform;
import '../ai/ai_settings_body.dart';
import '../ai/ai_store.dart';
import '../mounts/mounts.dart';
import '../sync/conflicts.dart';
import '../share/share_popup.dart' show shareClientOf;
import '../share/shared_links_settings.dart';
import '../sync/sync_settings.dart';
import '../updates/updates_settings.dart';
import 'connectors_body.dart';
import 'settings_kit.dart';
import 'settings_store.dart';

const appVersion = '0.1.0';

enum SettingsSection {
  general('General', Sx.general),
  reminders('Reminders', Sx.reminders),
  sync('Sync and devices', Sx.sync, group: 'Your vault'),
  conflicts('Conflicts', Sx.conflicts, group: 'Your vault'),
  storage('Storage', Sx.storage, group: 'Your vault'),
  sharedLinks('Shared links', Sx.link, group: 'Your vault'),
  agent('Agent', Sx.agent, group: 'AI'),
  connectors('Connectors', Sx.connectors, group: 'AI'),
  about('About', Sx.info);

  const SettingsSection(this.label, this.icon, {this.group});
  final String label;
  final Sx icon;

  /// The nav heading this page sits under, or null.
  final String? group;

  /// Shared links show only while this device is signed in to a hub.
  static List<SettingsSection> visibleFor(BuildContext context) => [
    for (final s in values)
      if (s != sharedLinks || shareClientOf(context) != null) s,
  ];
}

Widget sectionIcon(SettingsSection s, Color color, double size) =>
    SIcon(s.icon, size: size, color: color);

Widget sectionBody(SettingsSection s) => switch (s) {
  SettingsSection.general => const GeneralBody(),
  SettingsSection.reminders => const RemindersBody(),
  SettingsSection.sync => const _SyncAndDevicesBody(),
  SettingsSection.conflicts => const ConflictsBody(),
  SettingsSection.storage => const VaultBody(),
  SettingsSection.sharedLinks => const SharedLinksBody(),
  SettingsSection.agent => const AiSettingsBody(),
  SettingsSection.connectors => const ConnectorsBody(),
  SettingsSection.about => const AboutBody(),
};

/// Sync and devices: the hub this device syncs with, attachments and, on a
/// computer, the hub it runs itself with the devices paired to it. Add-on
/// sections placed with them ("Run schedules on this device") come last in
/// the hub group.
class _SyncAndDevicesBody extends StatelessWidget {
  const _SyncAndDevicesBody();

  @override
  Widget build(BuildContext context) {
    final extra = appAddonContext.settingsSections.all.where(
      (s) => s.location == 'addons' && (s.isVisible?.call(context) ?? true),
    );
    return SyncAndDevicesPage(extra: [for (final s in extra) s.build(context)]);
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
    for (final s in SettingsSection.visibleFor(context)) {
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
    for (final s in SettingsSection.visibleFor(context)) {
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

class GeneralBody extends StatelessWidget {
  const GeneralBody({super.key});

  static String _themeLabel(ThemePref t) => switch (t) {
    ThemePref.system => 'System',
    ThemePref.light => 'Light',
    ThemePref.dark => 'Dark',
  };

  @override
  Widget build(BuildContext context) {
    final s = AppSettings.of(context);
    final m = context.mobile;
    final phone = defaultTargetPlatform == TargetPlatform.android ||
        defaultTargetPlatform == TargetPlatform.iOS;
    return SPage(
      title: 'General',
      children: [
        SSection(
          title: 'Appearance',
          first: true,
          children: [
            SRow(
              label: 'Theme',
              desc: 'Light or dark colours, on this device only.',
              help:
                  'Leave it on System to follow Windows or Android, including their night schedule.',
              stack: m,
              control: SSegmented<ThemePref>(
                value: s.theme,
                options: ThemePref.values,
                labelOf: _themeLabel,
                keyOf: (t) => ValueKey('theme-${t.name}'),
                onChanged: (t) => s.update((s) => s.theme = t),
              ),
            ),
          ],
        ),
        SSection(
          title: 'Feedback',
          children: [
            SRow(
              label: 'Send feedback',
              desc:
                  'Write what happened. A screenshot and the app’s diagnostics go with it.',
              help: 'Whenever something breaks, looks wrong or confuses you.',
              stack: m,
              control: SButton(
                'Write feedback',
                key: const ValueKey('send-feedback'),
                icon: Sx.feedback,
                onTap: () => FeedbackHost.of(context).open(context),
              ),
            ),
            if (phone)
              SRow(
                label: 'Shake to send feedback',
                desc: 'Shake the phone to open the feedback sheet.',
                scope: 'Phone',
                help: 'Turn off if it opens by accident, for example while walking.',
                control: SSwitch(
                  key: const ValueKey('shake-to-send'),
                  label: 'Shake to send feedback',
                  value: s.shakeToSend,
                  onChanged: (on) => s.update((s) => s.shakeToSend = on),
                ),
              ),
          ],
        ),
        if (defaultTargetPlatform == TargetPlatform.android)
          SSection(
            title: 'Agent runs',
            children: [
              SRow(
                label: 'Notify when a run finishes or fails',
                desc:
                    'A notice when a task you delegated finishes or fails. It appears when the app syncs the change.',
                scope: 'Phone',
                control: SSwitch(
                  key: const ValueKey('run-notices'),
                  label: 'Notify when a run finishes or fails',
                  value: s.runNotices,
                  onChanged: (on) {
                    s.update((s) => s.runNotices = on);
                    if (on) unawaited(context.app.runNotices.enable());
                  },
                ),
              ),
            ],
          ),
      ],
    );
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
    final blocked = defaultTargetPlatform == TargetPlatform.android &&
        store.permissionGranted == false;
    final off = r.enabled ? null : 'Turn on Reminders to change this.';
    return SPage(
      title: 'Reminders',
      intro:
          'Notifications on this device for upcoming events, and one summary of your day each morning.',
      children: [
        if (blocked)
          SNotice(
            '**Notifications are off for $appName in Android settings.** Your reminder settings are kept; turn notifications on for $appName in Android’s settings to get them.',
            kind: SNoticeKind.warn,
            textKey: const ValueKey('reminders-permission-note'),
          ),
        SSection(
          first: !blocked,
          children: [
            SRow(
              label: 'Reminders',
              desc: 'Notify me about events and send a morning summary.',
              help:
                  'Turn off on a device where you don’t want to be notified, such as a shared computer.',
              control: SSwitch(
                key: const ValueKey('reminders-enabled'),
                label: 'Reminders',
                value: r.enabled,
                onChanged: (enabled) =>
                    s.update((s) => s.reminders = r.copyWith(enabled: enabled)),
              ),
            ),
            SRow(
              label: 'Remind me before events',
              desc: 'How early the notification comes.',
              disabled: !r.enabled,
              reason: off,
              control: SSelect<int>(
                key: const ValueKey('reminders-lead'),
                title: 'Remind me before events',
                value: r.leadMinutes,
                options: const [0, 5, 10, 30],
                labelOf: (minutes) =>
                    minutes == 0 ? 'At start' : '$minutes min before',
                onChanged: r.enabled
                    ? (lead) => s.update(
                        (s) => s.reminders = r.copyWith(leadMinutes: lead),
                      )
                    : null,
              ),
            ),
            SRow(
              label: 'Morning summary',
              desc: 'When the day’s summary arrives.',
              disabled: !r.enabled,
              reason: off,
              control: SSelect<int>(
                key: const ValueKey('reminders-morning'),
                title: 'Morning summary time',
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
          ],
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
    final s = AppSettings.of(context);
    final root = context.app.vault.root;
    final pending = s.vaultPath != null && s.vaultPath != root ? s.vaultPath : null;
    final desktop = !context.mobile;
    return SPage(
      title: 'Storage',
      intro:
          'Where this vault’s pages live on this device. Pages are Markdown files you can open with other apps.',
      children: [
        SSection(
          title: 'Vault folder',
          first: true,
          children: [
            SRow(
              label: 'Location',
              desc: 'The folder that holds your pages and their history.',
              stack: true,
              control: SCopyLine(root, textKey: const ValueKey('vault-path')),
              below: pending == null
                  ? null
                  : Padding(
                      padding: const EdgeInsets.only(top: 8),
                      child: Text(
                        '$appName opens $pending the next time it starts.',
                        key: const ValueKey('vault-pending'),
                        style: sHint(context),
                      ),
                    ),
            ),
            if (desktop)
              SAdvanced(
                headerKey: const ValueKey('vault-open-another'),
                title: 'Open another folder',
                summary: 'starts or opens a different vault',
                children: [
                  SRow(
                    label: 'Use a different folder',
                    desc:
                        'An empty folder starts a new vault; a vault folder opens it. Takes effect after a restart.',
                    help:
                        'To keep separate vaults (for example work and personal), or to move the vault to another drive.',
                    stack: true,
                    control: Row(children: [
                      Expanded(
                        child: STextField(
                          fieldKey: const ValueKey('vault-path-field'),
                          controller: _path,
                          mono: true,
                          placeholder: Platform.isWindows ? r'D:\Notes' : '/home/me/Notes',
                          onSubmitted: (_) => _use(s),
                        ),
                      ),
                      if (FileDialogs.instance.folders) ...[
                        const SizedBox(width: 8),
                        SButton('Choose…', key: const ValueKey('vault-choose'), onTap: () => _choose(s)),
                      ],
                      const SizedBox(width: 8),
                      SButton('Use folder', key: const ValueKey('vault-use'), onTap: () => _use(s)),
                    ]),
                    below: _error == null
                        ? null
                        : SNotice(_error!, kind: SNoticeKind.bad, textKey: const ValueKey('vault-error')),
                  ),
                  if (s.vaultPath != null)
                    SRow(
                      label: 'Default location',
                      desc: 'Go back to the folder $appName picked when it was installed.',
                      control: SButton(
                        'Use the default location',
                        key: const ValueKey('vault-use-default'),
                        onTap: () => s.update((s) => s.vaultPath = null),
                      ),
                    ),
                ],
              ),
          ],
        ),
        if (desktop) const MountsSection(),
      ],
    );
  }
}

class AboutBody extends StatelessWidget {
  const AboutBody({super.key});

  @override
  Widget build(BuildContext context) {
    final vault = context.app.vault;
    return SPage(
      title: 'About',
      children: [
        SSection(
          first: true,
          children: [
            const SRow(label: 'Version', desc: '$appName $appVersion'),
            SRow(
              label: 'Device ID',
              desc:
                  'Identifies this device to the hub and in conflicts. Support may ask for it.',
              stack: true,
              control: SCopyLine(vault.deviceId, textKey: const ValueKey('device-id')),
            ),
          ],
        ),
        // Windows only: the installed build and its updates, under the version.
        if (updatesAvailableHere()) const UpdatesBody(embedded: true),
      ],
    );
  }
}
