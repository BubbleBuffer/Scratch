// Settings > Agent > Installed packages: every runtime package this build
// lists for this computer (wiki/addons/packages.md) with its version and
// size on disk, Remove, and Download or Update where one is on offer. It
// reads the same [PackagesStore] as the pane's card, so progress is shared.
// Builds that list no package (and no other installed one) show nothing.

import 'package:flutter/widgets.dart';

import '../../services/addon_services.dart';
import 'packages_store.dart';

class PackagesSettings extends StatefulWidget {
  const PackagesSettings({super.key, required this.packages});
  final PackagesStore packages;

  @override
  State<PackagesSettings> createState() => _PackagesSettingsState();
}

class _PackagesSettingsState extends State<PackagesSettings> {
  @override
  void initState() {
    super.initState();
    widget.packages.ensureLoaded();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.packages,
    builder: (context, _) {
      final store = widget.packages;
      final shown = [
        for (final p in store.packages)
          if (p.state != PackageState.notAvailable) p,
      ];
      if (shown.isEmpty) return const SizedBox.shrink();
      return SSection(
        title: 'Installed packages',
        intro: 'Downloaded when needed, kept in this computer’s data folder.',
        children: [
          SList(
            children: [
              for (final p in shown) _PackageRow(store: store, info: p),
            ],
          ),
        ],
      );
    },
  );
}

class _PackageRow extends StatelessWidget {
  const _PackageRow({required this.store, required this.info});
  final PackagesStore store;
  final PackageInfo info;

  @override
  Widget build(BuildContext context) {
    final name = info.name;
    final status = store.statusOf(name);
    final working = status is PackageInstalling;
    final removing = store.isRemoving(name);
    final installed = info.state == PackageState.installed ||
        info.state == PackageState.updateAvailable;
    final download = packageSize(info.downloadBytes.toInt());
    final installedBytes = info.installedBytes;
    final String sub;
    if (status is PackageInstalling) {
      sub = switch (status.phase) {
        PackagePhase.downloading => 'Downloading ${status.percent}%',
        PackagePhase.verifying => 'Verifying',
        PackagePhase.extracting => 'Extracting',
        PackagePhase.installing => 'Installing',
      };
    } else if (removing) {
      sub = 'Removing';
    } else {
      sub = switch (info.state) {
        PackageState.installed =>
          'Version ${info.installedVersion ?? info.version}'
              '${installedBytes == null ? '' : ' · ${packageSize(installedBytes.toInt())}'}',
        PackageState.updateAvailable =>
          'Version ${info.installedVersion ?? '?'}'
              '${installedBytes == null ? '' : ' · ${packageSize(installedBytes.toInt())}'}'
              ' · update to ${info.version}, $download',
        _ => 'Not installed · $download download',
      };
    }
    final busy = store.installing || removing;
    return SItem(
      key: ValueKey('package-$name'),
      name: packageTitle(name),
      sub: sub,
      error: status is PackageFailed ? status.message : null,
      trailing: [
        if (info.state == PackageState.notInstalled)
          SButton(
            'Download',
            key: ValueKey('package-install-$name'),
            kind: SButtonKind.primary,
            onTap: busy ? null : () => store.install(name),
          ),
        if (info.state == PackageState.updateAvailable)
          SButton(
            'Update',
            key: ValueKey('package-install-$name'),
            kind: SButtonKind.primary,
            onTap: busy ? null : () => store.install(name),
          ),
        if (installed)
          SButton(
            'Remove',
            key: ValueKey('package-remove-$name'),
            onTap: busy || working ? null : () => store.remove(name),
          ),
      ],
    );
  }
}
