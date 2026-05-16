/// 底部 5 个 tab: 今日 / 客户 / 报价 / 合同 / 我.
/// 跟 web 版 TabBar 结构对齐. "煤池" 挪进 "我" 屏入口.
library;

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

class AppShell extends StatelessWidget {
  const AppShell({super.key, required this.navigationShell});

  final StatefulNavigationShell navigationShell;

  static const _tabs = [
    _Tab(icon: Icons.today_outlined, selected: Icons.today, label: '今日'),
    _Tab(
        icon: Icons.people_outline,
        selected: Icons.people,
        label: '客户'),
    _Tab(
        icon: Icons.request_quote_outlined,
        selected: Icons.request_quote,
        label: '报价'),
    _Tab(
        icon: Icons.assignment_outlined,
        selected: Icons.assignment,
        label: '合同'),
    _Tab(
        icon: Icons.person_outline, selected: Icons.person, label: '我'),
  ];

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: navigationShell,
      bottomNavigationBar: BottomNavigationBar(
        currentIndex: navigationShell.currentIndex,
        onTap: (i) => navigationShell.goBranch(
          i,
          initialLocation: i == navigationShell.currentIndex,
        ),
        items: [
          for (final t in _tabs)
            BottomNavigationBarItem(
              icon: Icon(t.icon),
              activeIcon: Icon(t.selected),
              label: t.label,
            ),
        ],
      ),
    );
  }
}

class _Tab {
  const _Tab({
    required this.icon,
    required this.selected,
    required this.label,
  });
  final IconData icon;
  final IconData selected;
  final String label;
}
