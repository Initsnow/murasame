#!/usr/bin/env python3
"""
Murasame Multi-Character One-Time Migration Script
将旧版单体 config.toml + chat.db 升级为模块化 characters/murasame 结构
"""
import os
import shutil
import sqlite3
import toml

def migrate():
    print("[*] 开始执行 Murasame 多角色架构一键迁移...")
    
    # 1. 检查旧配置文件
    if not os.path.exists("config.toml"):
        print("[-] 未检测到 config.toml，跳过迁移。")
        return

    with open("config.toml", "r", encoding="utf-8") as f:
        old_cfg = toml.load(f)

    # 2. 创建 characters/murasame 目录
    murasame_dir = os.path.join("characters", "murasame")
    os.makedirs(murasame_dir, exist_ok=True)
    
    # 3. 复制或准备默认头像
    avatar_dest = os.path.join(murasame_dir, "avatar.png")
    app_icon = os.path.join("assets", "icons", "app_icon.png")
    if os.path.exists(app_icon) and not os.path.exists(avatar_dest):
        shutil.copy(app_icon, avatar_dest)
        print(f"[+] 已复制默认角色头像至: {avatar_dest}")

    # 4. 生成 characters/murasame/character.toml
    char_toml_data = {
        "id": "murasame",
        "name": "丛雨",
        "avatar_path": "avatar.png",
        "model": old_cfg.get("model", {}),
        "audio": old_cfg.get("audio", {}),
        "ai": old_cfg.get("ai", {}),
        "cues": old_cfg.get("cues", {"mappings": {}, "expressions": {}, "motions": {}}),
    }

    char_toml_path = os.path.join(murasame_dir, "character.toml")
    with open(char_toml_path, "w", encoding="utf-8") as f:
        toml.dump(char_toml_data, f)
    print(f"[+] 已成功生成角色配置文件: {char_toml_path}")

    # 5. 生成精简的全新全局 config.toml
    new_global_cfg = {
        "active_character": "murasame",
        "window": old_cfg.get("window", {}),
        "user": {
            "name": "主人",
            "avatar_enabled": True,
            "avatar_path": "assets/user_avatar.png"
        },
        "ui": old_cfg.get("ui", {})
    }
    with open("config.toml", "w", encoding="utf-8") as f:
        toml.dump(new_global_cfg, f)
    print("[+] 已成功更新全局 config.toml 为现代精简配置")

    # 6. SQLite 数据库结构迁移
    if os.path.exists("chat.db"):
        conn = sqlite3.connect("chat.db")
        cur = conn.cursor()
        
        # 检查 sessions 是否已有 character_id 列
        cur.execute("PRAGMA table_info(sessions)")
        columns = [row[1] for row in cur.fetchall()]
        if "character_id" not in columns:
            print("[*] 正在为 chat.db 添加 character_id 字段...")
            cur.execute("ALTER TABLE sessions ADD COLUMN character_id TEXT NOT NULL DEFAULT 'murasame'")
            cur.execute("CREATE INDEX IF NOT EXISTS idx_sessions_character ON sessions(character_id)")
            conn.commit()
            print("[+] chat.db 迁移完成，所有历史记录已安全绑定至角色 'murasame'")
        else:
            print("[*] chat.db 已包含 character_id 字段。")
        conn.close()

    print("[√] 多角色架构迁移完成！现有项目已完全进入干净模块化状态。")

if __name__ == "__main__":
    migrate()
