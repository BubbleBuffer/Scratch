// The sync indicator in the top bar: a cloud icon (and on desktop a short
// label) for synced, syncing, offline and error. Tapping it shows the
// details (hub, last sync, the error) with "Sync now" and "Sync settings".
// Hidden while sync is off.

import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../state/sync_store.dart';
import '../../ui/ui.dart';
import '../settings/settings_screen.dart';
import 'certificate_view.dart';
import 'sync_icons.dart';

String phaseLabel(SyncPhase p) => switch (p) {
      SyncPhase.off => 'Sync off',
      SyncPhase.synced => 'Synced',
      SyncPhase.syncing => 'Syncing…',
      SyncPhase.offline => 'Offline',
      SyncPhase.error => 'Sync error',
    };

SyncIco phaseIcon(SyncPhase p) => switch (p) {
      SyncPhase.off => SyncIco.cloud,
      SyncPhase.synced => SyncIco.cloudCheck,
      SyncPhase.syncing => SyncIco.cloudSync,
      SyncPhase.offline => SyncIco.cloudOff,
      SyncPhase.error => SyncIco.cloudAlert,
    };

/// The indicator, reading the [SyncStore].
class SyncIndicator extends StatelessWidget {
  const SyncIndicator({super.key, this.showLabel});

  /// Show the label next to the icon (default: on desktop).
  final bool? showLabel;

  @override
  Widget build(BuildContext context) {
    final sync = context.watch<SyncStore>();
    final (phase, message) = sync.phase;
    if (phase == SyncPhase.off) return const SizedBox.shrink();
    return SyncIndicatorView(
      phase: phase,
      showLabel: showLabel ?? !context.sizeClass.isCompact,
      onTap: (anchor) => showSyncDetails(context, anchor, phase, message),
    );
  }
}

/// The indicator for a given [phase] (also used by the screenshots).
class SyncIndicatorView extends StatelessWidget {
  const SyncIndicatorView({super.key, required this.phase, this.showLabel = true, this.onTap});
  final SyncPhase phase;
  final bool showLabel;
  final void Function(Rect anchor)? onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.touch;
    final error = phase == SyncPhase.error;
    final color = error ? c.inlineCodeText : c.iconTertiary;
    return Builder(
      builder: (b) => Semantics(
        button: true,
        label: phaseLabel(phase),
        child: Hover(
          key: const ValueKey('sync-indicator'),
          onTap: onTap == null ? null : () => onTap!(globalRectOf(b)),
          radius: Radii.md,
          padding: EdgeInsets.symmetric(horizontal: showLabel ? Space.xs : 0),
          child: SizedBox(
            height: (m ? ControlSize.xl : ControlSize.sm).height,
            width: showLabel ? null : (m ? ControlSize.xl : ControlSize.sm).height,
            child: Row(mainAxisSize: MainAxisSize.min, mainAxisAlignment: MainAxisAlignment.center, children: [
              SyncIcon(phaseIcon(phase), size: m ? IconSize.xl : IconSize.md, color: color),
              if (showLabel) ...[
                const Gap(Space.xs),
                Text(phaseLabel(phase), style: ts(context.type.ui, error ? c.inlineCodeText : c.textTertiary, weight: Weight.regular)),
              ],
            ]),
          ),
        ),
      ),
    );
  }
}

/// The details popover (desktop) or sheet (mobile).
Future<void> showSyncDetails(BuildContext context, Rect anchor, SyncPhase phase, String? message) async {
  final sync = context.read<SyncStore>();
  Widget details(BuildContext ctx) {
    final c = ctx.colors;
    final m = ctx.touch;
    final lines = <String>[];
    if (sync.connected) {
      lines.add('Hub: ${sync.hubUrl}');
      final ok = sync.status.lastOkMs;
      if (ok != null) lines.add('Last synced ${relativeTime(ok.toInt()).toLowerCase()}');
    }
    if (sync.hosting) lines.add('Serving this vault on ${sync.hubAddress}');
    final cert = sync.untrustedCertificate;
    return Padding(
      padding: EdgeInsets.fromLTRB(m ? Layout.gutter : Space.sm, Space.xxs, m ? Layout.gutter : Space.sm, Space.sm),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
        Row(children: [
          SyncIcon(phaseIcon(phase), size: IconSize.md, color: phase == SyncPhase.error ? c.inlineCodeText : c.iconSecondary),
          const Gap(Space.sm),
          Text(phaseLabel(phase), style: ts(ctx.type.ui, c.textPrimary)),
        ]),
        if (cert) ...[
          const Gap(Space.sm),
          SyncCertificateProblem(
            messageKey: const ValueKey('sync-error-message'),
            onView: () => Navigator.of(ctx).pop('cert-view'),
            onTrust: () => Navigator.of(ctx).pop('cert-trust'),
          ),
        ] else if (message != null) ...[
          const Gap(Space.sm),
          Text(message, key: const ValueKey('sync-error-message'), style: ts(context.type.ui, c.textSecondary, weight: Weight.regular)),
        ],
        for (final l in lines) ...[
          const Gap(Space.xs),
          Text(l, style: ts(context.type.ui, c.textTertiary, weight: Weight.regular)),
        ],
      ]),
    );
  }

  final action = context.sizeClass.isCompact
      ? await showSheet<String>(
          context,
          (ctx) => SheetSurface(children: [
            details(ctx),
            if (sync.connected) SheetItem(label: 'Sync now', onTap: () => Navigator.of(ctx).pop('now')),
            SheetItem(label: 'Sync settings', icon: Ico.settings, onTap: () => Navigator.of(ctx).pop('settings')),
          ]),
        )
      : await showPopover<String>(
          context,
          anchor,
          (ctx) => MenuList(width: SyncSize.details, children: [
            details(ctx),
            const MenuSeparator(),
            if (sync.connected)
              MenuItem(key: const ValueKey('sync-now'), label: 'Sync now', onTap: () => Navigator.of(ctx).pop('now')),
            MenuItem(
                key: const ValueKey('sync-settings'),
                label: 'Sync settings',
                icon: Ico.settings,
                onTap: () => Navigator.of(ctx).pop('settings')),
          ]),
        );
  if (!context.mounted) return;
  switch (action) {
    case 'now':
      sync.syncNow();
    case 'cert-view':
      await showSyncCertificate(context);
    case 'cert-trust':
      await trustSyncCertificate(context);
    case 'settings':
      await showSettings(context, section: SettingsSection.sync);
  }
}
