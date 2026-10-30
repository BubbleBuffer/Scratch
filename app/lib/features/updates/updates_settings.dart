// The Updates settings section (Windows only) and the sidebar's "Update
// ready" row, which opens a small panel with the version, its release
// notes and "Restart to update". State: UpdateController.

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:url_launcher/url_launcher.dart';

import '../../ui/app_name.dart';
import '../../ui/ui.dart';
import '../settings/controls.dart';
import 'update_controller.dart';

/// Whether the Updates section and indicator exist on this device.
bool updatesAvailableHere() =>
    defaultTargetPlatform == TargetPlatform.windows && UpdateController.current != null;

String _two(int n) => n.toString().padLeft(2, '0');

String _checkedAt(DateTime t, DateTime now) {
  final time = '${_two(t.hour)}:${_two(t.minute)}';
  if (t.year == now.year && t.month == now.month && t.day == now.day) return 'today at $time';
  return '${t.year}-${_two(t.month)}-${_two(t.day)} $time';
}

String _version(String tag) => tag.startsWith('v') ? tag.substring(1) : tag;

String _channel(String tag) {
  if (tag.isEmpty) return 'Development build';
  if (isNightlyTag(tag)) return 'Nightly';
  return tag.contains('-') ? 'Pre-release' : 'Release';
}

/// The one-line status for [u].
String updateStatusText(UpdateController u, {DateTime? now}) {
  final r = u.release;
  return switch (u.phase) {
    UpdatePhase.development => 'Development build: updates are off.',
    UpdatePhase.idle => u.checkedAt == null ? 'Not checked yet.' : 'Checked ${_checkedAt(u.checkedAt!, now ?? u.now())}.',
    UpdatePhase.checking => 'Checking for updates…',
    UpdatePhase.upToDate => 'Up to date. Checked ${_checkedAt(u.checkedAt!, now ?? u.now())}.',
    UpdatePhase.available => 'Version ${_version(r!.tag)} is available.',
    UpdatePhase.downloading => u.progress == null
        ? 'Downloading version ${_version(r!.tag)}…'
        : 'Downloading version ${_version(r!.tag)}… ${(u.progress! * 100).round()}%',
    UpdatePhase.ready => 'Version ${_version(r!.tag)} is ready to install.',
    UpdatePhase.installing => 'Starting the installer…',
  };
}

Future<void> _openNotes(String url) async {
  final uri = Uri.tryParse(url);
  if (uri == null || !uri.isScheme('https')) return;
  await launchUrl(uri, mode: LaunchMode.externalApplication);
}

class UpdatesBody extends StatelessWidget {
  const UpdatesBody({super.key});

  @override
  Widget build(BuildContext context) {
    final u = UpdateController.current;
    if (u == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: Listenable.merge([u, u.settings]),
      builder: (context, _) => _body(context, u),
    );
  }

  Widget _body(BuildContext context, UpdateController u) {
    final s = u.settings;
    final c = context.colors;
    final caption = tsRaw(13, 18, 400, c.textSecondary);
    final r = u.release;
    Widget? action;
    if (u.phase == UpdatePhase.available) {
      action = TextBtn('Download', key: const ValueKey('updates-download'), onTap: u.download);
    } else if (u.phase == UpdatePhase.ready) {
      action = TextBtn('Restart to update', key: const ValueKey('updates-install'), onTap: u.install);
    }
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const SettingsHeading('Updates', primary: true),
      SettingsRow(
        label: 'Version',
        description: u.enabled ? '$appName ${_version(u.currentTag)} · ${_channel(u.currentTag)}' : '$appName · Development build',
      ),
      SettingsRow(
        label: 'Include nightly builds',
        description: 'Daily builds of the latest changes. Less tested than releases.',
        trailing: Toggle(
          key: const ValueKey('updates-nightly'),
          value: s.updatesIncludeNightly,
          onChanged: u.enabled
              ? (on) {
                  s.update((s) => s.updatesIncludeNightly = on);
                  u.check();
                }
              : null,
        ),
      ),
      SettingsRow(
        label: 'Download updates automatically',
        description: 'Updates download in the background. They install only when you choose.',
        trailing: Toggle(
          key: const ValueKey('updates-auto-download'),
          value: s.updatesAutoDownload,
          onChanged: u.enabled ? (on) => s.update((s) => s.updatesAutoDownload = on) : null,
        ),
      ),
      SettingsRow(
        label: 'Status',
        description: updateStatusText(u),
        trailing: TextBtn(
          'Check now',
          key: const ValueKey('updates-check-now'),
          onTap: u.enabled && !u.busy ? u.check : null,
        ),
      ),
      if (r != null && action != null)
        SettingsRow(
          label: '${r.nightly ? 'Nightly' : 'Version'} ${_version(r.tag)}',
          description: 'Published ${r.publishedAt.length >= 10 ? r.publishedAt.substring(0, 10) : r.publishedAt}.',
          trailing: Row(mainAxisSize: MainAxisSize.min, children: [
            if (r.notesUrl.isNotEmpty)
              TextBtn('Release notes', key: const ValueKey('updates-notes'), color: c.textSecondary, onTap: () => _openNotes(r.notesUrl)),
            const SizedBox(width: 8),
            action,
          ]),
        ),
      if (u.problem != null && u.phase != UpdatePhase.development)
        Padding(
          padding: const EdgeInsets.only(top: 8),
          child: Text(u.problem!, key: const ValueKey('updates-problem'), style: caption),
        ),
    ]);
  }
}

/// The sidebar row shown while an update is downloaded and ready.
class UpdateRailRow extends StatelessWidget {
  const UpdateRailRow({super.key});

  @override
  Widget build(BuildContext context) {
    final u = UpdateController.current;
    if (u == null || defaultTargetPlatform != TargetPlatform.windows) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: u,
      builder: (context, _) {
        if (!u.ready) return const SizedBox.shrink();
        final c = context.colors;
        return Padding(
          padding: const EdgeInsets.fromLTRB(8, 4, 8, 0),
          child: AppTooltip(
            label: 'Version ${_version(u.release!.tag)} is ready to install',
            child: Hover(
              key: const ValueKey('update-ready-row'),
              onTap: () => showUpdateReady(context, u),
              borderRadius: Radii.ctl,
              hoverColor: c.rail2.withValues(alpha: .55),
              padding: const EdgeInsets.symmetric(horizontal: 8),
              child: SizedBox(
                height: Layout.sidebarRowHeight - 2,
                child: Row(children: [
                  Container(
                    width: 8,
                    height: 8,
                    decoration: BoxDecoration(color: c.accent, borderRadius: BorderRadius.circular(4)),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text('Update ready', maxLines: 1, overflow: TextOverflow.ellipsis, style: tsRaw(14, 20, 400, c.railInk)),
                  ),
                ]),
              ),
            ),
          ),
        );
      },
    );
  }
}

/// The panel behind the "Update ready" row.
Future<void> showUpdateReady(BuildContext context, UpdateController u) => showModalPanel<void>(
      context,
      (ctx) => _UpdateReadyBox(u: u, onClose: () => Navigator.of(ctx).pop()),
      width: 340,
      shrink: true,
    );

class _UpdateReadyBox extends StatelessWidget {
  const _UpdateReadyBox({required this.u, required this.onClose});
  final UpdateController u;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    return ListenableBuilder(
      listenable: u,
      builder: (context, _) {
        final r = u.release;
        return Padding(
          padding: const EdgeInsets.fromLTRB(20, 20, 20, 16),
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            Text('Update ready', textAlign: TextAlign.center, style: tsDisplay(const TypeSpec(19, 24, 640), c.textPrimary)),
            const SizedBox(height: 6),
            Text(
              r == null
                  ? updateStatusText(u)
                  : '$appName ${_version(r.tag)} is downloaded and verified. $appName closes, the installer runs, then $appName opens again.',
              textAlign: TextAlign.center,
              style: ts(ty.uiSmall, c.textSecondary, weight: 400),
            ),
            if (r != null && r.notesUrl.isNotEmpty) ...[
              const SizedBox(height: 8),
              Center(
                child: TextBtn('Release notes', key: const ValueKey('update-ready-notes'), onTap: () => _openNotes(r.notesUrl)),
              ),
            ],
            if (u.problem != null && u.phase == UpdatePhase.available) ...[
              const SizedBox(height: 8),
              Text(u.problem!, textAlign: TextAlign.center, style: ts(ty.uiSmall, c.inlineCodeText, weight: 400)),
            ],
            const SizedBox(height: 16),
            DialogButton(
              key: const ValueKey('update-ready-install'),
              label: u.phase == UpdatePhase.installing ? 'Starting…' : 'Restart to update',
              filled: true,
              onTap: u.ready ? u.install : null,
            ),
            const SizedBox(height: 8),
            DialogButton(key: const ValueKey('update-ready-later'), label: 'Later', onTap: onClose),
          ]),
        );
      },
    );
  }
}
