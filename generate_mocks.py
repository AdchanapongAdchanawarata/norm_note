import os

vault_dir = "/Users/adchanapong/Desktop/norm_note/norm_ui_vault"

folders = [
    "Work/Projects",
    "Work/Meetings",
    "Personal/Journal",
    "Personal/Travel",
    "Ideas/App Concepts"
]

for f in folders:
    os.makedirs(os.path.join(vault_dir, f), exist_ok=True)

notes = [
    {
        "path": "Work/Projects/01_Product_Launch.md",
        "content": "# Q4 Product Launch 🚀\n\nWe need to make sure the launch is flawless. The new UI is looking great.\n\n## Tasks\n- [x] Finalize landing page\n- [ ] Update screenshots\n- [ ] Send newsletter\n\n![Team Sync](https://images.unsplash.com/photo-1522071820081-009f0129c71c?w=800&q=80)\n\n#work #urgent #launch"
    },
    {
        "path": "Work/Meetings/Weekly_Sync.md",
        "content": "# Weekly Team Sync\n\n**Date:** Oct 24, 2024\n\n### Agenda\n1. Review last week's metrics.\n2. Discuss new feature roadmap.\n\n> \"Speed is a feature.\"\n\n#work #meeting"
    },
    {
        "path": "Personal/Journal/2024_10_24.md",
        "content": "# Today's Thoughts 🌿\n\nI went for a run this morning. The weather was perfect. I'm feeling really productive today.\n\n![Nature](https://images.unsplash.com/photo-1510798831971-661eb04b3739?w=800&q=80)\n\n#personal #journal #mindfulness"
    },
    {
        "path": "Personal/Travel/Japan_Trip_2025.md",
        "content": "# Japan Trip 2025 🇯🇵\n\nPlanning the itinerary for Tokyo and Kyoto. \n\n## Places to Visit\n- Shibuya Crossing\n- Fushimi Inari Shrine\n- Akihabara\n\n#travel #japan #planning"
    },
    {
        "path": "Ideas/App Concepts/NextGen_Editor.md",
        "content": "# NextGen Markdown Editor\n\nThe goal is to build something incredibly fast, local-first, and distraction-free.\n\n```rust\nfn main() {\n    println!(\"Hello, world!\");\n}\n```\n\n#ideas #coding #rust"
    },
    {
        "path": "02_Dashboard.md",
        "content": "# Welcome to your Dashboard\n\nThis is the central hub for everything.\n\n- **Work:** 2 active projects.\n- **Personal:** 1 upcoming trip.\n\n![Dashboard](https://images.unsplash.com/photo-1499951360447-b19be8fe80f5?w=800&q=80)\n\n#dashboard #overview #pinned"
    }
]

for note in notes:
    full_path = os.path.join(vault_dir, note["path"])
    with open(full_path, "w", encoding="utf-8") as f:
        f.write(note["content"])
    print(f"Created {full_path}")
