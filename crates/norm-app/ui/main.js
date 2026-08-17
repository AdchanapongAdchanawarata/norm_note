const invoke = window.__TAURI__?.core?.invoke || window.__TAURI__?.invoke;
const listen = window.__TAURI__?.event?.listen;

// OS detection for keyboard shortcuts
const isMac = navigator.platform.toUpperCase().indexOf('MAC') >= 0;
document.addEventListener('DOMContentLoaded', () => {
    if (isMac) {
        document.documentElement.classList.add('is-mac');
    } else {
        document.documentElement.classList.add('is-windows');
    }
    const modSymbol = isMac ? '⌘' : 'Ctrl+';
    const modText = isMac ? 'Cmd' : 'Ctrl';
    
    document.querySelectorAll('.mod-sym').forEach(el => el.textContent = modSymbol);
    document.querySelectorAll('.mod-text').forEach(el => el.textContent = modText);
    
    const undoBtn = document.getElementById('undo-btn');
    if (undoBtn) undoBtn.title = `Undo (${modText}+Z)`;
    
    const redoBtn = document.getElementById('redo-btn');
    if (redoBtn) redoBtn.title = `Redo (${modText}+Y)`;
});

// showAlert must be defined before handleMenuAction which calls it
function showAlert(message) {
    return new Promise((resolve) => {
        const overlay = document.getElementById('custom-modal-overlay');
        const msgEl = document.getElementById('modal-message');
        const inputEl = document.getElementById('modal-input');
        const okBtn = document.getElementById('modal-ok-btn');
        const cancelBtn = document.getElementById('modal-cancel-btn');
        
        if (!overlay || !msgEl || !okBtn) {
            console.error("Modal elements not found!");
            resolve(true);
            return;
        }

        msgEl.textContent = message;
        inputEl.style.display = 'none';
        cancelBtn.style.display = 'none';
        overlay.style.display = 'flex';
        
        const cleanup = () => {
            overlay.style.display = 'none';
            cancelBtn.style.display = 'inline-block';
            okBtn.removeEventListener('click', onOk);
        };
        
        const onOk = () => { cleanup(); resolve(true); };
        
        okBtn.addEventListener('click', onOk);
    });
}
window.showAlert = showAlert;

window.onerror = function(message, source, lineno, colno, error) {
    if (invoke) {
        invoke('save_note', { id: 'js_errors.log', content: message + ' at ' + lineno + ':' + colno + '\n' + (error ? error.stack : '') });
    }
};

window.addEventListener("unhandledrejection", function(event) {
    if (invoke) {
        invoke('save_note', { id: 'js_errors.log', content: 'Unhandled rejection: ' + event.reason });
    }
});



// Context Menu State
let contextMenuTargetId = null;
let contextMenuTargetType = null;

document.addEventListener('DOMContentLoaded', () => {
    const ctxMenu = document.getElementById('context-menu');
    
    document.addEventListener('click', () => {
        if (ctxMenu) ctxMenu.style.display = 'none';
    });

    document.getElementById('ctx-rename')?.addEventListener('click', () => {
        if (contextMenuTargetType === 'note') {
            const item = document.querySelector(`.note-item[data-id="${contextMenuTargetId.replace(/"/g, '\\"')}"]`);
            item?.querySelector('.rename-btn')?.click();
        } else if (contextMenuTargetType === 'folder') {
            const btn = document.querySelector(`.folder-header[data-folder="${contextMenuTargetId.replace(/"/g, '\\"')}"] .rename-folder-btn`);
            btn?.click();
        }
    });
    
    document.getElementById('ctx-duplicate')?.addEventListener('click', () => {
        if (contextMenuTargetType === 'note') {
            const item = document.querySelector(`.note-item[data-id="${contextMenuTargetId.replace(/"/g, '\\"')}"]`);
            item?.querySelector('.dup-note-btn')?.click();
        } else if (contextMenuTargetType === 'folder') {
            const btn = document.querySelector(`.folder-header[data-folder="${contextMenuTargetId.replace(/"/g, '\\"')}"] .dup-folder-btn`);
            btn?.click();
        }
    });

    document.getElementById('ctx-delete')?.addEventListener('click', () => {
        if (contextMenuTargetType === 'note') {
            const item = document.querySelector(`.note-item[data-id="${contextMenuTargetId.replace(/"/g, '\\"')}"]`);
            item?.querySelector('.del-note-btn')?.click();
        } else if (contextMenuTargetType === 'folder') {
            const btn = document.querySelector(`.folder-header[data-folder="${contextMenuTargetId.replace(/"/g, '\\"')}"] .del-btn`);
            btn?.click();
        }
    });

    document.getElementById('ctx-move')?.addEventListener('click', async () => {
        if (contextMenuTargetType === 'note') {
            const folder = await showPrompt('Move to folder:', '');
            if (folder !== null) await moveNoteToFolder(contextMenuTargetId, folder.trim());
        } else if (contextMenuTargetType === 'folder') {
            const targetFolder = await showPrompt('Merge folder into:', '');
            if (targetFolder !== null && targetFolder.trim() !== '') await mergeFolderToFolder(contextMenuTargetId, targetFolder.trim());
        }
    });

    document.getElementById('focus-btn')?.addEventListener('click', () => {
        document.body.classList.toggle('focus-mode');
    });
});

function showContextMenu(e, id, type) {
    e.preventDefault();
    contextMenuTargetId = id;
    contextMenuTargetType = type;
    const ctxMenu = document.getElementById('context-menu');
    
    ctxMenu.style.display = 'block';
    
    let x = e.clientX;
    let y = e.clientY;
    if (x + ctxMenu.offsetWidth > window.innerWidth) x -= ctxMenu.offsetWidth;
    if (y + ctxMenu.offsetHeight > window.innerHeight) y -= ctxMenu.offsetHeight;
    
    ctxMenu.style.left = `${x}px`;
    ctxMenu.style.top = `${y}px`;
}

function handleMenuAction(action) {
    if (invoke) {
        invoke('save_note', { id: 'js_menu.log', content: action }).catch(err => {});
    }
    if (action === "new-note") {
        document.getElementById('new-note-btn')?.click();
    } else if (action === "new-folder") {
        document.getElementById('new-folder-btn')?.click();
    } else if (action === "import-files") {
        document.getElementById('import-btn')?.click();
    } else if (action === "settings") {
        document.getElementById('open-settings-btn')?.click();
    } else if (action === "export-note") {
        document.getElementById('export-btn')?.click();
    } else if (action === "delete-note") {
        const delBtn = document.querySelector('.note-item.active .del-note-btn');
        if (delBtn) delBtn.click();
    } else if (action === "rename-note") {
        const renBtn = document.querySelector('.note-item.active .rename-btn');
        if (renBtn) renBtn.click();
    } else if (action === "duplicate-note") {
        const dupBtn = document.querySelector('.note-item.active .dup-note-btn');
        if (dupBtn) dupBtn.click();
    } else if (action === "toggle-sidebar") {
        const sidebar = document.querySelector('.sidebar');
        if (sidebar) {
            sidebar.classList.toggle('collapsed');
        }
    } else if (action === "backup-vault") {
        document.getElementById('backup-vault-btn')?.click();
    } else if (action === "restore-vault") {
        document.getElementById('restore-vault-btn')?.click();
    } else if (action.startsWith("open-recent:")) {
        const noteId = action.split("open-recent:")[1];
        if (noteId) {
            loadNote(noteId);
        }
    } else if (action === "zoom-in") {
        const currentZoom = parseFloat(document.body.style.zoom || 1);
        document.body.style.zoom = currentZoom + 0.1;
    } else if (action === "zoom-out") {
        const currentZoom = parseFloat(document.body.style.zoom || 1);
        document.body.style.zoom = Math.max(0.5, currentZoom - 0.1);
    } else if (action === "actual-size") {
        document.body.style.zoom = 1;
    } else if (action === "help-doc") {
        const helpModal = document.getElementById('help-modal');
        const searchInput = document.getElementById('help-search-input');
        if (helpModal) {
            helpModal.style.display = 'flex';
            if (searchInput) searchInput.focus();
        }
    }
}

if (listen) {
    listen("menu-event", (e) => {
        handleMenuAction(e.payload);
    });
}

window.addEventListener('native-menu', (e) => {
    handleMenuAction(e.detail);
});

// Sync recent notes to native menu on startup
try {
    const recent = JSON.parse(localStorage.getItem('recent_notes') || '[]');
    if (window.__TAURI__ && recent.length > 0) {
        invoke('update_recent_menu', { recentNotes: recent }).catch(e => console.error(e));
    }
} catch(e) {}


const notesListEl = document.getElementById('notes-list');
const noteTitleEl = document.getElementById('note-title');
const noteBodyEl = document.getElementById('note-body');
const newNoteBtn = document.getElementById('new-note-btn');
const importBtn = document.getElementById('import-btn');

let currentNotes = [];
let activeNoteId = null;
let selectedNoteIds = new Set();
let lastSelectedNoteId = null;
let currentFolder = "";
let folderState = {};

function updateBulkActionBar() {
    const actionBar = document.getElementById('bulk-action-bar');
    const bulkCount = document.getElementById('bulk-count');
    const mergeBtn = document.getElementById('bulk-merge-btn');
    if (selectedNoteIds.size > 1) {
        actionBar.style.display = 'flex';
        bulkCount.innerText = `${selectedNoteIds.size} selected`;
        if (mergeBtn) {
            mergeBtn.style.opacity = '1';
            mergeBtn.style.pointerEvents = 'auto';
        }
    } else {
        actionBar.style.display = 'none';
    }
}

document.getElementById('bulk-delete-btn').addEventListener('click', async () => {
    if (selectedNoteIds.size <= 1) return;
    if (await showConfirm(`Delete ${selectedNoteIds.size} notes?`)) {
        const deletedNotes = [];
        for (const id of selectedNoteIds) {
            const note = currentNotes.find(n => n.id === id);
            if (note) {
                try {
                    const content = await invoke('read_note', { id: id });
                    await invoke('delete_note', { id: id });
                    deletedNotes.push({ note: { ...note }, content: content });
                    currentNotes = currentNotes.filter(n => n.id !== id);
                    if (activeNoteId === id) activeNoteId = null;
                } catch(e) {}
            }
        }
        
        pushSidebarAction({ type: 'BATCH_DELETE', notes: deletedNotes });
        
        if (!activeNoteId && currentNotes.length > 0) selectNote(currentNotes[0].id);
        else if (!activeNoteId) { noteTitleEl.value = ''; noteBodyEl.value = ''; }
        
        selectedNoteIds.clear();
        updateBulkActionBar();
        renderNotes();
    }
});

document.getElementById('bulk-merge-btn').addEventListener('click', async () => {
    if (selectedNoteIds.size <= 1) return;
    if (await showConfirm(`Merge ${selectedNoteIds.size} notes into the first selected note?`)) {
        const ids = Array.from(selectedNoteIds);
        const targetId = ids[0]; // Merge into the first selected
        
        try {
            let oldTargetContent = await invoke('read_note', { id: targetId });
            let mergedContent = oldTargetContent;
            const deletedNotes = [];
            
            for (let i = 1; i < ids.length; i++) {
                const srcId = ids[i];
                const srcNote = currentNotes.find(n => n.id === srcId);
                if (srcNote) {
                    const content = await invoke('read_note', { id: srcId });
                    mergedContent += `\n\n---\n\n## ${srcNote.title || 'Untitled'}\n\n${content}`;
                    await invoke('delete_note', { id: srcId });
                    deletedNotes.push({ note: { ...srcNote }, content: content });
                    currentNotes = currentNotes.filter(n => n.id !== srcId);
                    if (activeNoteId === srcId) activeNoteId = null;
                }
            }
            
            await invoke('save_note', { id: targetId, content: mergedContent });
            
            const targetNote = currentNotes.find(n => n.id === targetId);
            if (targetNote) {
                const parts = mergedContent.split('\n\n');
                targetNote.title = parts[0] ? parts[0].replace(/^# /, '') : 'Untitled Note';
                targetNote.preview = parts.slice(1).join('\n').substring(0, 50).replace(/\n/g, ' ');
            }
            
            pushSidebarAction({ type: 'BATCH_MERGE', targetId, oldTargetContent, newTargetContent: mergedContent, deletedNotes });
            
            if (activeNoteId === targetId) {
                noteBodyEl.value = mergedContent;
            } else if (!activeNoteId && currentNotes.length > 0) {
                selectNote(currentNotes[0].id);
            }
            
            selectedNoteIds.clear();
            selectedNoteIds.add(targetId);
            lastSelectedNoteId = targetId;
            
            updateBulkActionBar();
            renderNotes();
        } catch(e) {
            console.error('Failed to batch merge', e);
        }
    }
});

// Bulk Duplicate Logic
document.getElementById('bulk-duplicate-btn').addEventListener('click', async () => {
    if (selectedNoteIds.size === 0) return;
    try {
        const ids = Array.from(selectedNoteIds);
        const duplicatedNotes = [];
        for (const id of ids) {
            const note = currentNotes.find(n => n.id === id);
            if (!note) continue;
            const content = await invoke('read_note', { id: id });
            const newId = id + ' (copy)';
            await invoke('save_note', { id: newId, content: content });
            const newNote = { id: newId, updated: Date.now(), title: note.title + ' (copy)', preview: note.preview, tags: note.tags ? [...note.tags] : [] };
            currentNotes.push(newNote);
            
            const oldIndex = noteOrder.indexOf(id);
            if (oldIndex !== -1) noteOrder.splice(oldIndex + 1, 0, newId);
            else noteOrder.push(newId);
            
            duplicatedNotes.push({ note: newNote, content: content });
        }
        saveNoteOrder();
        
        pushSidebarAction({ type: 'BATCH_DUPLICATE', notes: duplicatedNotes });
        
        selectedNoteIds.clear();
        updateBulkActionBar();
        renderNotes();
    } catch(e) { console.error('Batch duplicate failed', e); }
});

// Bulk Move Logic
const bulkMoveBtn = document.getElementById('bulk-move-btn');
const bulkMoveDropdown = document.getElementById('bulk-move-dropdown');
if (bulkMoveBtn && bulkMoveDropdown) {
    bulkMoveBtn.onclick = (e) => {
        e.stopPropagation();
        const isOpen = bulkMoveDropdown.style.display === 'block';
        if (!isOpen) {
            // Populate folders
            const folders = new Set();
            currentNotes.forEach(n => {
                if (n.id.includes('/')) folders.add(n.id.substring(0, n.id.lastIndexOf('/')));
            });
            let html = `<div class="dropdown-item" data-folder="/">/ (Root)</div>`;
            Array.from(folders).sort().forEach(f => {
                html += `<div class="dropdown-item" data-folder="${f}">${f}</div>`;
            });
            bulkMoveDropdown.innerHTML = html;
            
            bulkMoveDropdown.querySelectorAll('.dropdown-item').forEach(item => {
                item.onclick = async (ev) => {
                    ev.stopPropagation();
                    bulkMoveDropdown.style.display = 'none';
                    const targetFolder = ev.target.dataset.folder;
                    const dest = targetFolder === '/' ? '' : targetFolder;
                    
                    const batchMoves = [];
                    const ids = Array.from(selectedNoteIds);
                    for (const id of ids) {
                        const currentFolder = id.includes('/') ? id.substring(0, id.lastIndexOf('/')) : '';
                        if (currentFolder !== dest) {
                            const res = await moveNoteToFolder(id, dest === '' ? '/' : dest, false);
                            if (res) batchMoves.push(res);
                        }
                    }
                    if (batchMoves.length > 0) {
                        pushSidebarAction({ type: 'BATCH_MOVE', moves: batchMoves });
                    }
                    selectedNoteIds.clear();
                    updateBulkActionBar();
                    renderNotes();
                };
            });
        }
        bulkMoveDropdown.style.display = isOpen ? 'none' : 'block';
    };
    
    document.addEventListener('click', (e) => {
        if (bulkMoveDropdown.style.display === 'block' && !bulkMoveDropdown.contains(e.target) && e.target !== bulkMoveBtn) {
            bulkMoveDropdown.style.display = 'none';
        }
    });
}

let saveTimeout = null;

function createNoteItem(note) {
    const item = document.createElement('div');
    item.dataset.id = note.id;
    item.className = 'note-item' + (note.id === activeNoteId ? ' active' : '') + (selectedNoteIds.has(note.id) ? ' selected' : '');
    if (note.tags) item.dataset.tags = note.tags.join(' ');
    
    item.addEventListener('contextmenu', (e) => showContextMenu(e, note.id, 'note'));
    
    item.draggable = true;
    item.ondragstart = (e) => {
        if (!selectedNoteIds.has(note.id)) {
            selectedNoteIds.clear();
            selectedNoteIds.add(note.id);
            updateBulkActionBar();
            document.querySelectorAll('.note-item.selected').forEach(el => el.classList.remove('selected'));
            item.classList.add('selected');
        }
        
        const payload = JSON.stringify(Array.from(selectedNoteIds));
        const rawData = 'MULTINOTE::' + payload;
        window.draggedItemRawData = rawData;
        e.dataTransfer.setData('text/plain', rawData);
        e.dataTransfer.effectAllowed = 'move';
        item.style.opacity = '0.5';
    };
    item.ondragend = () => {
        window.draggedItemRawData = null;
        item.style.opacity = '1';
    };
    item.ondragenter = (e) => {
        e.preventDefault();
    };
    item.ondragover = (e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = 'move';
        
        const rect = item.getBoundingClientRect();
        const y = e.clientY - rect.top;
        item.classList.remove('drop-before', 'drop-after');
        item.style.backgroundColor = '';
        
        if (y < rect.height * 0.25) {
            item.classList.add('drop-before');
        } else if (y > rect.height * 0.75) {
            item.classList.add('drop-after');
        } else {
            item.style.backgroundColor = 'var(--hover-bg)';
        }
    };
    item.ondragleave = () => {
        item.classList.remove('drop-before', 'drop-after');
        item.style.backgroundColor = '';
    };
    item.ondrop = async (e) => {
        e.preventDefault();
        e.stopPropagation();
        
        const rect = item.getBoundingClientRect();
        const y = e.clientY - rect.top;
        item.classList.remove('drop-before', 'drop-after');
        item.style.backgroundColor = '';
        
        const rawData = e.dataTransfer.getData('text/plain') || window.draggedItemRawData;
        if (rawData) {
            if (y < rect.height * 0.25) {
                await handleSidebarDrop(note.id, rawData, 'before');
            } else if (y > rect.height * 0.75) {
                await handleSidebarDrop(note.id, rawData, 'after');
            } else {
                if (rawData.startsWith('NOTE::')) {
                    const srcNoteId = rawData.substring(6);
                    if (srcNoteId !== note.id) mergeNoteToNote(srcNoteId, note.id);
                }
            }
        }
    };
    
    item.dataset.tags = note.tags ? note.tags.join(' ') : '';
    item.innerHTML = `
        <div class="note-item-prefix" style="display: flex; align-items: center; gap: 8px; margin-right: 12px; margin-left: 2px; flex-shrink: 0; color: #b0b0b0;">
            <div class="checkbox-wrapper" style="display: flex; align-items: center; justify-content: center; width: 24px; height: 24px; cursor: pointer; margin-left: -6px; margin-right: -2px;">
                <input type="checkbox" class="note-checkbox" ${selectedNoteIds.has(note.id) ? 'checked' : ''} style="pointer-events: none;" />
            </div>
            <div class="star-btn ${note.tags && note.tags.includes('pinned') ? 'starred' : ''}" style="cursor:pointer; display:flex; align-items:center; justify-content: center; width: 12px; height: 12px; ${note.tags && note.tags.includes('pinned') ? 'color: var(--accent-color);' : ''}" title="Pin note">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="${note.tags && note.tags.includes('pinned') ? 'currentColor' : 'none'}" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"></path></svg>
            </div>
        </div>
        <div class="note-item-content" style="flex: 1; min-width: 0;">
            <div class="note-title" title="${(note.title || 'Untitled Note').replace(/"/g, '&quot;')}">${note.title || 'Untitled Note'}</div>
            <div class="note-preview">${note.preview || '...'}</div>
        </div>
        <div class="item-actions">
            <button class="inline-btn dup-note-btn" title="Duplicate Note">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>
            </button>
            <button class="inline-btn rename-btn" title="Rename Note">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"></path><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"></path></svg>
            </button>
            <button class="inline-btn del-note-btn" title="Delete Note">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"></polyline><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path></svg>
            </button>
        </div>
    `;
    
    item.onclick = (e) => {
        if (e.target.closest('.item-actions')) return;
        
        if (e.metaKey || e.ctrlKey) {
            if (selectedNoteIds.has(note.id)) {
                selectedNoteIds.delete(note.id);
            } else {
                selectedNoteIds.add(note.id);
                lastSelectedNoteId = note.id;
            }
            updateBulkActionBar();
            renderNotes();
            return;
        }
        
        if (e.shiftKey && lastSelectedNoteId) {
            const flatIds = Array.from(notesListEl.querySelectorAll('.note-item')).map(el => el.dataset.id);
            const idx1 = flatIds.indexOf(lastSelectedNoteId);
            const idx2 = flatIds.indexOf(note.id);
            if (idx1 !== -1 && idx2 !== -1) {
                const start = Math.min(idx1, idx2);
                const end = Math.max(idx1, idx2);
                selectedNoteIds.clear();
                for (let i = start; i <= end; i++) {
                    selectedNoteIds.add(flatIds[i]);
                }
            }
            updateBulkActionBar();
            renderNotes();
            return;
        }
        
        selectedNoteIds.clear();
        selectedNoteIds.add(note.id);
        lastSelectedNoteId = note.id;
        updateBulkActionBar();
        
        selectNote(note.id);
    };
    
    const checkboxWrapper = item.querySelector('.checkbox-wrapper');
    const checkbox = item.querySelector('.note-checkbox');
    checkboxWrapper.onclick = (e) => {
        e.stopPropagation();
        checkbox.checked = !checkbox.checked;
        if (checkbox.checked) {
            selectedNoteIds.add(note.id);
            lastSelectedNoteId = note.id;
        } else {
            selectedNoteIds.delete(note.id);
        }
        updateBulkActionBar();
        item.classList.toggle('selected', checkbox.checked);
    };
    
    const starBtn = item.querySelector('.star-btn');
    starBtn.onclick = async (e) => {
        e.stopPropagation();
        try {
            let content = await invoke('read_note', { id: note.id });
            const hasPinned = note.tags && note.tags.includes('pinned');
            
            if (hasPinned) {
                content = content.replace(/\s*#pinned\b/g, '');
                note.tags = note.tags.filter(t => t !== 'pinned');
                starBtn.style.color = '';
                starBtn.querySelector('svg').setAttribute('fill', 'none');
            } else {
                content = content.trim() + '\n\n#pinned';
                if (!note.tags) note.tags = [];
                note.tags.push('pinned');
                starBtn.style.color = 'var(--accent-color)';
                starBtn.querySelector('svg').setAttribute('fill', 'currentColor');
            }
            
            item.dataset.tags = note.tags.join(' ');
            await invoke('save_note', { id: note.id, content: content });
            
            if (activeNoteId === note.id) {
                noteBodyEl.value = content;
            }
        } catch (err) {
            console.error('Failed to toggle star', err);
        }
    };
    
    const dupBtn = item.querySelector('.dup-note-btn');
        if (dupBtn) {
            dupBtn.onclick = async (e) => {
                e.stopPropagation();
                try {
                    const content = await invoke('read_note', { id: note.id });
                    const newId = note.id + ' (copy)';
                    await invoke('save_note', { id: newId, content: content });
                    const newNote = { id: newId, updated: Date.now(), title: note.title + ' (copy)', preview: note.preview };
                    currentNotes.push(newNote);
                    
                    // Duplicate ordering
                    const oldIndex = noteOrder.indexOf(note.id);
                    if (oldIndex !== -1) noteOrder.splice(oldIndex + 1, 0, newId);
                    else noteOrder.push(newId);
                    saveNoteOrder();
                    
                    pushSidebarAction({ type: 'DUPLICATE_NOTE', oldId: note.id, newId: newId });
                    renderNotes();
                } catch (err) { console.error('Failed to duplicate', err); }
            };
        }
        
        const renameBtn = item.querySelector('.rename-btn');
    if (renameBtn) {
        renameBtn.onclick = async (e) => {
            e.stopPropagation();
            const parts = note.id.split('/');
            const currentFilename = parts[parts.length - 1];
            const currentName = currentFilename.replace('.md', '');
            
            const defaultName = (note.title && note.title.trim() !== '' && note.title !== 'Untitled Note') ? note.title : 'Untitled Note';
            const newName = await showPrompt('Enter new note name:', defaultName);
            
            if (newName && newName.trim() !== "") {
                const folder = parts.slice(0, -1).join('/');
                const newId = folder ? `${folder}/${newName.trim()}.md` : `${newName.trim()}.md`;
                if (newId === note.id) return;
                
                try {
                    const content = await invoke('read_note', { id: note.id });
                    
                    let newContent = content;
                    const lines = content.split('\n');
                    if (lines.length > 0 && lines[0].startsWith('# ')) {
                        lines[0] = '# ' + newName.trim();
                        newContent = lines.join('\n');
                    } else {
                        newContent = '# ' + newName.trim() + '\n\n' + content;
                    }
                    
                    await invoke('save_note', { id: newId, content: newContent });
                    await invoke('delete_note', { id: note.id });
                    const oldId = note.id;
                    note.id = newId;
                    note.title = newName.trim();
                    if (activeNoteId === oldId) activeNoteId = newId;
                    
                    const idx = noteOrder.indexOf(oldId);
                    if (idx !== -1) {
                        noteOrder[idx] = newId;
                        saveNoteOrder();
                    }
                    
                    pushSidebarAction({ type: 'RENAME_NOTE', oldId: oldId, newId: newId });
                    renderNotes();
                } catch (err) {
                    console.error("Failed to rename note", err);
                }
            }
        };
    }


    const delBtn = item.querySelector('.del-note-btn');
    if (delBtn) {
        delBtn.onclick = async (e) => {
            e.stopPropagation();
            if (await showConfirm('Delete this note?')) {
                try {
                    const content = await invoke('read_note', { id: note.id });
                    await invoke('delete_note', { id: note.id });
                    currentNotes = currentNotes.filter(n => n.id !== note.id);
                    if (activeNoteId === note.id) {
                        activeNoteId = null;
                        if (currentNotes.length > 0) selectNote(currentNotes[0].id);
                        else { noteTitleEl.value = ''; noteBodyEl.value = ''; }
                    }
                    pushSidebarAction({ type: 'DELETE_NOTE', note: { ...note }, content: content });
                    renderNotes();
                } catch (err) {
                    console.error('Failed to delete', err);
                }
            }
        };
    }
    
    return item;
}

async function mergeFolderToFolder(srcFolder, targetFolder) {
    if (!srcFolder || !targetFolder || srcFolder === targetFolder) return;
    
    // Find all notes in srcFolder
    const notesInFolder = currentNotes.filter(n => {
        const parts = n.id.split('/');
        if (parts.length > 1) {
            const f = parts.slice(0, -1).join('/');
            return f === srcFolder;
        }
        return false;
    });
    
    for (const note of notesInFolder) {
        await moveNoteToFolder(note.id, targetFolder);
    }
    renderNotes();
}

async function renameFolder(oldFolder, newFolder, pushHistory = true) {
    if (!oldFolder || !newFolder || oldFolder === newFolder) return;
    
    const notesInFolder = currentNotes.filter(n => {
        const parts = n.id.split('/');
        if (parts.length > 1) {
            const f = parts.slice(0, -1).join('/');
            return f === oldFolder;
        }
        return false;
    });
    
    for (const note of notesInFolder) {
        const parts = note.id.split('/');
        const filename = parts[parts.length - 1];
        const newId = `${newFolder}/${filename}`;
        
        try {
            const content = await invoke('read_note', { id: note.id });
            await invoke('save_note', { id: newId, content: content });
            await invoke('delete_note', { id: note.id });
            
            note.id = newId;
            if (activeNoteId === note.id) activeNoteId = newId;
        } catch (err) {
            console.error("Failed to rename note in folder", err);
        }
    }
    renderNotes();
}

let activeTag = null;

function renderTags() {
    const tagsContainer = document.getElementById('tags-container');
    if (!tagsContainer) return;
    
    if (tagsContainer) tagsContainer.innerHTML = '';
    
    // Extract unique tags
    const allTags = new Set();
    currentNotes.forEach(note => {
        if (note.tags && Array.isArray(note.tags)) {
            note.tags.forEach(tag => allTags.add(tag));
        }
    });
    
    const tags = Array.from(allTags).sort();
    
    if (tags.length === 0) {
        tagsContainer.style.display = 'none';
        return;
    }
    
    tagsContainer.style.display = 'flex';
    
    tags.forEach(tag => {
        const tagEl = document.createElement('div');
        tagEl.className = 'tag-item';
        tagEl.innerText = tag;
        tagEl.style.padding = '2px 8px';
        tagEl.style.borderRadius = '12px';
        tagEl.style.fontSize = '12px';
        tagEl.style.cursor = 'pointer';
        tagEl.style.background = activeTag === tag ? 'var(--accent)' : 'var(--bg-color)';
        tagEl.style.color = activeTag === tag ? '#fff' : 'var(--text-color)';
        tagEl.style.border = '1px solid ' + (activeTag === tag ? 'var(--accent)' : 'var(--border-color)');
        
        tagEl.onclick = (e) => {
            e.stopPropagation();
            if (activeTag === tag) {
                activeTag = null; // deselect
            } else {
                activeTag = tag;
            }
            renderNotes();
        };
        
        tagsContainer.appendChild(tagEl);
    });
}

function renderNotes() {
    notesListEl.innerHTML = '';
    
    renderTags();
    
    const groups = { '/': [] };
    
    currentNotes.forEach(note => {
        // Tag filter
        if (activeTag && (!note.tags || !note.tags.includes(activeTag))) {
            return;
        }
        
        const parts = note.id.split('/');
        if (parts.length > 1) {
            const folder = parts.slice(0, -1).join('/');
            if (!groups[folder]) groups[folder] = [];
            groups[folder].push(note);
        } else {
            groups['/'].push(note);
        }
    });
    
    // Helper to get sort index
    const getSortIndex = (id) => {
        const idx = noteOrder.indexOf(id);
        return idx === -1 ? Number.MAX_SAFE_INTEGER : idx;
    };
    
    // Render root notes
    groups['/'].sort((a, b) => getSortIndex(a.id) - getSortIndex(b.id)).forEach(note => {
        notesListEl.appendChild(createNoteItem(note));
    });
    
    // Render folders
    Object.keys(groups).sort((a, b) => {
        if (a === '/') return 0;
        return getSortIndex('FOLDER::' + a) - getSortIndex('FOLDER::' + b);
    }).forEach(folder => {
        if (folder === '/') return;
        
        const groupEl = document.createElement('div');
        groupEl.className = 'folder-group';
        
        const isCollapsed = folderState[folder] === false;
        const hasActive = isCollapsed && activeNoteId && activeNoteId.startsWith(folder + '/');
        
        const headerEl = document.createElement('div');
        headerEl.className = 'folder-header' + (isCollapsed ? ' collapsed' : '') + (hasActive ? ' active' : '');
        headerEl.dataset.folder = folder;
        headerEl.addEventListener('contextmenu', (e) => showContextMenu(e, folder, 'folder'));
        const folderDisplayName = folder.split('/').pop();
        headerEl.innerHTML = `
            <div style="display: flex; align-items: center; flex: 1; min-width: 0;" title="${folder.replace(/"/g, '&quot;')}">
                <span class="drag-handle" title="Drag to move folder" style="cursor: grab; margin-right: 5px; display: flex; align-items: center; opacity: 0.5;">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="9" cy="5" r="1"></circle><circle cx="9" cy="12" r="1"></circle><circle cx="9" cy="19" r="1"></circle><circle cx="15" cy="5" r="1"></circle><circle cx="15" cy="12" r="1"></circle><circle cx="15" cy="19" r="1"></circle></svg>
                </span>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="margin-right: 4px;">
                    <polyline points="6 9 12 15 18 9"></polyline>
                </svg>
                <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">${folderDisplayName}</span>
            </div>
            <div class="item-actions">
                <button class="inline-btn dup-folder-btn" title="Duplicate Folder">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>
                </button>
                <button class="inline-btn rename-folder-btn" title="Rename Folder">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"></path><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"></path></svg>
                </button>
                <button class="inline-btn add-btn" title="New Note">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"></line><line x1="5" y1="12" x2="19" y2="12"></line></svg>
                </button>
                <button class="inline-btn del-btn" title="Delete Folder">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"></polyline><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path></svg>
                </button>
            </div>
        `;
        
        const contentEl = document.createElement('div');
        contentEl.className = 'folder-content' + (folderState[folder] === false ? ' collapsed' : '');
        
        // Drop target for folders
        headerEl.ondragenter = (e) => {
            e.preventDefault();
        };
        headerEl.ondragover = (e) => {
            e.preventDefault();
            e.dataTransfer.dropEffect = 'move';
            
            const rect = headerEl.getBoundingClientRect();
            const y = e.clientY - rect.top;
            headerEl.classList.remove('drop-before', 'drop-after');
            headerEl.classList.remove('drag-over');
            
            if (y < rect.height * 0.25) {
                headerEl.classList.add('drop-before');
            } else if (y > rect.height * 0.75) {
                headerEl.classList.add('drop-after');
            } else {
                headerEl.classList.add('drag-over');
            }
        };
        headerEl.ondragleave = () => {
            headerEl.classList.remove('drop-before', 'drop-after');
            headerEl.classList.remove('drag-over');
        };
        headerEl.ondrop = async (e) => {
            e.preventDefault();
            e.stopPropagation();
            
            const rect = headerEl.getBoundingClientRect();
            const y = e.clientY - rect.top;
            headerEl.classList.remove('drop-before', 'drop-after');
            headerEl.classList.remove('drag-over');
            
            const rawData = e.dataTransfer.getData('text/plain') || window.draggedItemRawData;
            if (rawData) {
                if (rawData.startsWith('MULTINOTE::')) {
                    const ids = JSON.parse(rawData.substring(11));
                    const batchMoves = [];
                    for (const id of ids) {
                        const currentFolder = id.includes('/') ? id.substring(0, id.lastIndexOf('/')) : '';
                        if (currentFolder !== folder) {
                            const res = await moveNoteToFolder(id, folder, false);
                            if (res) batchMoves.push(res);
                        }
                    }
                    if (batchMoves.length > 0) {
                        pushSidebarAction({ type: 'BATCH_MOVE', moves: batchMoves });
                    }
                    selectedNoteIds.clear();
                    updateBulkActionBar();
                    renderNotes();
                    return;
                }
                
                if (y < rect.height * 0.25) {
                    await handleSidebarDrop('FOLDER::' + folder, rawData, 'before');
                } else if (y > rect.height * 0.75) {
                    await handleSidebarDrop('FOLDER::' + folder, rawData, 'after');
                } else {
                    if (rawData.startsWith('NOTE::')) {
                        const noteId = rawData.substring(6);
                        await moveNoteToFolder(noteId, folder);
                    } else if (rawData.startsWith('FOLDER::')) {
                        const srcFolder = rawData.substring(8);
                        if (srcFolder !== folder) await mergeFolderToFolder(srcFolder, folder);
                    }
                }
            }
        };
        
        headerEl.draggable = true;
        headerEl.ondragstart = (e) => {
            const rawData = 'FOLDER::' + folder;
            window.draggedItemRawData = rawData;
            e.dataTransfer.setData('text/plain', rawData);
            e.dataTransfer.effectAllowed = 'move';
            headerEl.style.opacity = '0.5';
        };
        headerEl.ondragend = () => {
            window.draggedItemRawData = null;
            headerEl.style.opacity = '1';
        };

        headerEl.onclick = () => {
            const isCollapsed = contentEl.classList.toggle('collapsed');
            headerEl.classList.toggle('collapsed', isCollapsed);
            folderState[folder] = !isCollapsed;
            if (!isCollapsed) currentFolder = folder;
            
            const hasActive = isCollapsed && activeNoteId && activeNoteId.startsWith(folder + '/');
            headerEl.classList.toggle('active', !!hasActive);
        };
        
        const dupFolderBtn = headerEl.querySelector('.dup-folder-btn');
        if (dupFolderBtn) {
            dupFolderBtn.onclick = async (e) => {
                e.stopPropagation();
                try {
                    const newFolder = folder + ' (copy)';
                    // Duplicate all notes inside
                    for (const note of groups[folder]) {
                        const filename = note.id.split('/').pop();
                        const newId = newFolder + '/' + filename;
                        const content = await invoke('read_note', { id: note.id });
                        await invoke('save_note', { id: newId, content: content });
                        currentNotes.push({ id: newId, updated: Date.now(), title: note.title, preview: note.preview });
                    }
                    
                    const oldIndex = noteOrder.indexOf('FOLDER::' + folder);
                    if (oldIndex !== -1) noteOrder.splice(oldIndex + 1, 0, 'FOLDER::' + newFolder);
                    else noteOrder.push('FOLDER::' + newFolder);
                    saveNoteOrder();
                    
                    pushSidebarAction({ type: 'DUPLICATE_FOLDER', oldFolder: folder, newFolder: newFolder });
                    renderNotes();
                } catch (err) { console.error('Failed to duplicate folder', err); }
            };
        }

        const renameFolderBtn = headerEl.querySelector('.rename-folder-btn');
        if (renameFolderBtn) {
            renameFolderBtn.onclick = async (e) => {
                e.stopPropagation();
                const newName = await showPrompt('Enter new folder name:', folder);
                if (newName && newName.trim() !== folder) {
                    renameFolder(folder, newName.trim().replace(/\s+/g, '_'));
                }
            };
        }
        
        const addNoteBtn = headerEl.querySelector('.add-btn');
        if (addNoteBtn) {
            addNoteBtn.onclick = (e) => {
                e.stopPropagation();
                currentFolder = folder;
                folderState[folder] = true;
                newNoteBtn.click();
            };
        }
        
        const delFolderBtn = headerEl.querySelector('.del-btn');
        if (delFolderBtn) {
            delFolderBtn.onclick = async (e) => {
                e.stopPropagation();
                if (await showConfirm(`Delete folder '${folder}' and all notes inside?`)) {
                    const deletedNotes = [];
                    for (const note of groups[folder]) {
                        try {
                            const content = await invoke('read_note', { id: note.id });
                            await invoke('delete_note', { id: note.id });
                            deletedNotes.push({ note: { ...note }, content: content });
                            currentNotes = currentNotes.filter(n => n.id !== note.id);
                            if (activeNoteId === note.id) activeNoteId = null;
                        } catch (err) {
                            console.error('Failed to delete note', note.id, err);
                        }
                    }
                    pushSidebarAction({ type: 'DELETE_FOLDER', folder: folder, notes: deletedNotes });
                    if (!activeNoteId && currentNotes.length > 0) selectNote(currentNotes[0].id);
                    else if (!activeNoteId) { noteTitleEl.value = ''; noteBodyEl.value = ''; }
                    renderNotes();
                }
            };
        }
        
        // Render notes in folder
        groups[folder].sort((a, b) => getSortIndex(a.id) - getSortIndex(b.id)).forEach(note => {
            contentEl.appendChild(createNoteItem(note));
        });
        
        groupEl.appendChild(headerEl);
        groupEl.appendChild(contentEl);
        notesListEl.appendChild(groupEl);
    });
}

function trackRecentNote(id, title) {
    try {
        let recent = JSON.parse(localStorage.getItem('recent_notes') || '[]');
        // Remove if exists
        recent = recent.filter(n => n[0] !== id);
        // Add to front
        recent.unshift([id, title]);
        if (recent.length > 10) {
            recent.pop();
        }
        localStorage.setItem('recent_notes', JSON.stringify(recent));
        if (window.__TAURI__) {
            invoke('update_recent_menu', { recentNotes: recent }).catch(e => console.error(e));
        }
    } catch (e) {
        console.error("Failed to track recent note", e);
    }
}

async function selectNote(id) {
    if (activeNoteId === id) return;
    activeNoteId = id;
    renderNotes();
    
    try {
        const content = await invoke('read_note', { id: id });
        const lines = content.split('\n');
        
        let displayTitle = id;
        if (lines.length > 0 && lines[0].startsWith('# ')) {
            noteTitleEl.value = lines[0].replace('# ', '');
            noteBodyEl.value = lines.slice(1).join('\n').trimStart();
            displayTitle = noteTitleEl.value;
        } else {
            noteTitleEl.value = '';
            noteBodyEl.value = content;
        }
        
        trackRecentNote(id, displayTitle);
        
        if (isPreviewMode) {
            setPreviewMode(true);
        }
    } catch (e) {
        console.error('Failed to read note', e);
        noteTitleEl.value = '';
        noteBodyEl.value = '';
    }
}

async function loadNotes() {
    try {
        currentNotes = await invoke('get_notes');
        
        updateTagsUI();
        
        renderNotes(); // Always render first to populate the DOM
        if (currentNotes.length > 0 && !activeNoteId) {
            selectNote(currentNotes[0].id);
        }
    } catch (e) {
        console.error('Failed to load notes', e);
    }
}

// History Stack
class HistoryStack {
    constructor() {
        this.undoStack = [];
        this.redoStack = [];
        this.lastState = "";
    }
    push(state) {
        if (state !== this.lastState) {
            this.undoStack.push(this.lastState);
            this.lastState = state;
            this.redoStack = [];
        }
    }
    undo() {
        if (this.undoStack.length > 0) {
            this.redoStack.push(this.lastState);
            this.lastState = this.undoStack.pop();
            return this.lastState;
        }
        return null;
    }
    redo() {
        if (this.redoStack.length > 0) {
            this.undoStack.push(this.lastState);
            this.lastState = this.redoStack.pop();
            return this.lastState;
        }
        return null;
    }
    reset(state) {
        this.undoStack = [];
        this.redoStack = [];
        this.lastState = state;
    }
}
const textHistory = new HistoryStack();

// Move note logic
async function moveNoteToFolder(noteId, targetFolder, pushHistory = true) {
    const parts = noteId.split('/');
    const filename = parts[parts.length - 1];
    
    const newFolderStr = targetFolder === '/' ? '' : targetFolder;
    const newId = newFolderStr ? `${newFolderStr}/${filename}` : filename;
    
    if (newId === noteId) return null;
    
    const note = currentNotes.find(n => n.id === noteId);
    if (!note) return null;
    
    try {
        const content = await invoke('read_note', { id: noteId });
        await invoke('save_note', { id: newId, content: content });
        await invoke('delete_note', { id: noteId });
        
        note.id = newId;
        if (activeNoteId === noteId) {
            activeNoteId = newId;
        }
        
        const orderIdx = noteOrder.indexOf(noteId);
        if (orderIdx !== -1) {
            noteOrder[orderIdx] = newId;
            saveNoteOrder();
        }
        
        const oldFolder = noteId === filename ? '/' : noteId.substring(0, noteId.length - filename.length - 1);
        
        // Push to history
        if (pushHistory) {
            pushSidebarAction({ type: 'MOVE_NOTE', noteId: newId, oldFolder: oldFolder, newFolder: targetFolder });
        }
        
        renderNotes();
        return { noteId: newId, oldFolder: oldFolder, newFolder: targetFolder };
    } catch (err) {
        console.error("Failed to move note", err);
        showAlert("Failed to move note: " + err);
    }
}


async function mergeNoteToNote(srcId, dstId) {
    try {
        const srcContent = await invoke('read_note', { id: srcId });
        const dstContent = await invoke('read_note', { id: dstId });
        
        const lines = srcContent.split('\n');
        if (lines[0].startsWith('#')) lines.shift();
        
        const mergedContent = dstContent + '\n\n---\n\n' + lines.join('\n').trim();
        
        await invoke('save_note', { id: dstId, content: mergedContent });
        await invoke('delete_note', { id: srcId });
        
        const srcNoteObj = currentNotes.find(n => n.id === srcId);
        pushSidebarAction({ type: 'MERGE_NOTE', srcId: srcId, dstId: dstId, srcContent: srcContent, dstContent: dstContent, srcNoteObj: srcNoteObj ? Object.assign({}, srcNoteObj) : null });
        
        currentNotes = currentNotes.filter(n => n.id !== srcId);
        
        const dstNoteObj = currentNotes.find(n => n.id === dstId);
        if (dstNoteObj) {
            const parts = mergedContent.split('\n\n');
            dstNoteObj.title = parts[0].replace(/^# /, '');
            dstNoteObj.preview = parts.slice(1).join('\n').substring(0, 50).replace(/\n/g, ' ');
        }
        
        if (activeNoteId === srcId) {
            activeNoteId = null;
        }
        if (activeNoteId === dstId) {
            const parts = mergedContent.split('\n\n');
            const title = parts[0].replace(/^# /, '');
            const body = parts.slice(1).join('\n\n');
            noteTitleEl.value = title;
            noteBodyEl.value = body;
            textHistory.reset(body);
        }
        renderNotes();
    } catch (err) {
        console.error("Failed to merge notes", err);
    }
}

async function mergeFolderToFolder(srcFolder, dstFolder) {
    if (srcFolder === dstFolder) return;
    const notesToMove = currentNotes.filter(n => n.id.startsWith(srcFolder + '/'));
    if (notesToMove.length === 0) return;
    
    if (await showConfirm(`Merge folder '${srcFolder}' into '${dstFolder}'?`)) {
        const batchMoves = [];
        for (const note of notesToMove) {
            const res = await moveNoteToFolder(note.id, dstFolder, false);
            if (res) batchMoves.push(res);
        }
        if (batchMoves.length > 0) {
            pushSidebarAction({ type: 'BATCH_MOVE', moves: batchMoves });
        }
        renderNotes();
    }
}

// Allow dropping onto the sidebar to move to root
const sidebarEl = document.querySelector('.sidebar');
sidebarEl.ondragenter = (e) => e.preventDefault();
sidebarEl.ondragover = (e) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
};
sidebarEl.ondrop = async (e) => {
    e.preventDefault();
    if (e.target.closest('.folder-group') === null && e.target.closest('.note-item') === null) {
        const rawData = e.dataTransfer.getData('text/plain') || window.draggedItemRawData;
        if (rawData) {
            if (rawData.startsWith('NOTE::')) {
                const noteId = rawData.substring(6);
                moveNoteToFolder(noteId, '/');
            } else if (rawData.startsWith('MULTINOTE::')) {
                const ids = JSON.parse(rawData.substring(11));
                const batchMoves = [];
                for (const id of ids) {
                    const currentFolder = id.includes('/') ? id.substring(0, id.lastIndexOf('/')) : '';
                    if (currentFolder !== '') {
                        const res = await moveNoteToFolder(id, '/', false);
                        if (res) batchMoves.push(res);
                    }
                }
                if (batchMoves.length > 0) {
                    pushSidebarAction({ type: 'BATCH_MOVE', moves: batchMoves });
                }
                selectedNoteIds.clear();
                updateBulkActionBar();
                renderNotes();
            }
        }
    }
};

if (newNoteBtn) newNoteBtn.onclick = async () => {
    const filename = Date.now().toString() + ".md";
    const id = currentFolder ? `${currentFolder}/${filename}` : filename;
    const newNote = {
        id: id,
        title: 'Untitled Note',
        preview: '...',
        updated_at: Date.now()
    };
    
    currentNotes.unshift(newNote);
    await invoke('save_note', { id: id, content: "# Untitled Note\n\n" });
    selectNote(id);
    noteTitleEl.focus();
    noteTitleEl.select();
};

const importDropdown = document.getElementById('import-dropdown');

if (importBtn && importDropdown) {
    importBtn.onclick = (e) => {
        e.stopPropagation();
        importDropdown.style.display = importDropdown.style.display === 'none' ? 'block' : 'none';
        
        // Hide export dropdown if open
        const expDropdown = document.getElementById('export-dropdown');
        if (expDropdown) expDropdown.style.display = 'none';
    };

    document.addEventListener('click', (e) => {
        if (importDropdown.style.display === 'block' && !importDropdown.contains(e.target) && e.target !== importBtn) {
            importDropdown.style.display = 'none';
        }
    });

    document.querySelectorAll('#import-dropdown .dropdown-item').forEach(item => {
        item.onclick = async (e) => {
            e.stopPropagation();
            importDropdown.style.display = 'none';
            const format = e.target.dataset.format;
            
            try {
                const importedIds = await invoke('import_files_dialog', { 
                    folder: currentFolder || '',
                    filterType: format
                });
                
                if (importedIds && importedIds.length > 0) {
                    const notes = await invoke('get_notes');
                    currentNotes = notes;
                    renderNotes();
                    if (importedIds[0] && importedIds[0].endsWith('.md')) {
                        selectNote(importedIds[0]);
                    }
                    showAlert('Done! Import successfully completed.');
                }
            } catch (err) {
                console.error('Import failed', err);
            }
        };
    });
}

function scheduleSave() {
    if (saveTimeout) clearTimeout(saveTimeout);
    saveTimeout = setTimeout(async () => {
        const title = noteTitleEl.value.trim() || 'Untitled Note';
        const body = noteBodyEl.value;
        const content = `# ${title}\n\n${body}`;

        if (!activeNoteId) {
            if (!title && !body) return; // Don't save empty state
            const filename = Date.now().toString() + ".md";
            activeNoteId = currentFolder ? `${currentFolder}/${filename}` : filename;
            const newNote = {
                id: activeNoteId,
                title: title,
                preview: body.substring(0, 50).replace(/\n/g, ' '),
                updated_at: Date.now()
            };
            currentNotes.unshift(newNote);
        }
        
        try {
            await invoke('save_note', { id: activeNoteId, content: content });
            // Update preview
            const note = currentNotes.find(n => n.id === activeNoteId);
            if (note) {
                note.title = title;
                note.preview = body.substring(0, 50).replace(/\n/g, ' ');
                
                // Parse tags
                const tags = new Set();
                const words = content.split(/\s+/);
                words.forEach(w => {
                    if (w.startsWith('#') && w.length > 1) {
                        const tag = w.replace(/[^\w]/g, '');
                        if (tag) tags.add(tag);
                    }
                });
                note.tags = Array.from(tags).sort();
                
                updateTagsUI();
                
                // Update DOM directly instead of full renderNotes() to prevent lag
                const activeEl = document.querySelector(`.note-item[data-id="${CSS.escape(activeNoteId)}"]`);
                if (activeEl) {
                    const tEl = activeEl.querySelector('.note-title');
                    const pEl = activeEl.querySelector('.note-preview');
                    if (tEl) tEl.textContent = title;
                    if (pEl) pEl.textContent = note.preview;
                }
            }
        } catch (e) {
            console.error('Failed to save note', e);
        }
    }, 1500); // 1500ms debounce
}

noteTitleEl.addEventListener('input', scheduleSave);
noteTitleEl.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
        e.preventDefault();
        noteBodyEl.focus();
    }
});

noteBodyEl.addEventListener('input', () => {
    scheduleSave();
    textHistory.push(noteBodyEl.value);
});

noteBodyEl.addEventListener('keydown', (e) => {
    // 1. Tab / Shift+Tab for indentation
    if (e.key === 'Tab') {
        e.preventDefault();
        const start = noteBodyEl.selectionStart;
        const end = noteBodyEl.selectionEnd;
        const val = noteBodyEl.value;
        
        if (start === end) {
            // No selection
            if (!e.shiftKey) {
                // Insert 4 spaces
                noteBodyEl.value = val.substring(0, start) + '    ' + val.substring(end);
                noteBodyEl.selectionStart = noteBodyEl.selectionEnd = start + 4;
            } else {
                // Remove up to 4 spaces before cursor
                const lineStart = val.lastIndexOf('\n', start - 1) + 1;
                const beforeCursor = val.substring(lineStart, start);
                const spaceMatch = beforeCursor.match(/ {1,4}$/);
                if (spaceMatch) {
                    const removeLen = spaceMatch[0].length;
                    noteBodyEl.value = val.substring(0, start - removeLen) + val.substring(end);
                    noteBodyEl.selectionStart = noteBodyEl.selectionEnd = start - removeLen;
                }
            }
        } else {
            // Multi-line indent/unindent
            const startLine = val.lastIndexOf('\n', start - 1) + 1;
            const endLine = val.indexOf('\n', end);
            const actualEnd = endLine === -1 ? val.length : endLine;
            const lines = val.substring(startLine, actualEnd).split('\n');
            
            let newLines;
            let startOffset = 0;
            let totalLengthChange = 0;
            
            if (!e.shiftKey) {
                newLines = lines.map((line, idx) => {
                    if (idx === 0) startOffset = 4;
                    totalLengthChange += 4;
                    return '    ' + line;
                });
            } else {
                newLines = lines.map((line, idx) => {
                    const match = line.match(/^ {1,4}/);
                    const removed = match ? match[0].length : 0;
                    if (idx === 0) startOffset = -removed;
                    totalLengthChange -= removed;
                    return line.substring(removed);
                });
            }
            
            noteBodyEl.value = val.substring(0, startLine) + newLines.join('\n') + val.substring(actualEnd);
            noteBodyEl.selectionStart = Math.max(startLine, start + startOffset);
            noteBodyEl.selectionEnd = end + totalLengthChange;
        }
        scheduleSave();
        textHistory.push(noteBodyEl.value);
        return;
    }
    
    // 2. Cmd/Ctrl key shortcuts
    if (e.ctrlKey || e.metaKey) {
        const start = noteBodyEl.selectionStart;
        const end = noteBodyEl.selectionEnd;
        const val = noteBodyEl.value;

        // Duplicate Line/Selection
        if (e.key.toLowerCase() === 'd') {
            e.preventDefault();
            if (start === end) {
                // Duplicate current line
                const lineStart = val.lastIndexOf('\n', start - 1) + 1;
                const lineEnd = val.indexOf('\n', start);
                const actualEnd = lineEnd === -1 ? val.length : lineEnd;
                const lineText = val.substring(lineStart, actualEnd);
                
                noteBodyEl.value = val.substring(0, actualEnd) + '\n' + lineText + val.substring(actualEnd);
                noteBodyEl.selectionStart = noteBodyEl.selectionEnd = actualEnd + 1 + (start - lineStart);
            } else {
                // Duplicate selection
                const selected = val.substring(start, end);
                noteBodyEl.value = val.substring(0, end) + selected + val.substring(end);
                noteBodyEl.selectionStart = end;
                noteBodyEl.selectionEnd = end + selected.length;
            }
            scheduleSave();
            textHistory.push(noteBodyEl.value);
            return;
        }
        
        // Merge Lines
        if (e.key.toLowerCase() === 'j') {
            e.preventDefault();
            if (start === end) {
                // Join current line with next line
                const lineEnd = val.indexOf('\n', start);
                if (lineEnd !== -1) {
                    let nextLineStart = lineEnd + 1;
                    while (nextLineStart < val.length && (val[nextLineStart] === ' ' || val[nextLineStart] === '\t')) {
                        nextLineStart++;
                    }
                    noteBodyEl.value = val.substring(0, lineEnd) + ' ' + val.substring(nextLineStart);
                    noteBodyEl.selectionStart = noteBodyEl.selectionEnd = lineEnd + 1;
                }
            } else {
                // Join all lines in selection
                const before = val.substring(0, start);
                const selected = val.substring(start, end);
                const after = val.substring(end);
                
                const joined = selected.replace(/\n\s*/g, ' ');
                noteBodyEl.value = before + joined + after;
                noteBodyEl.selectionStart = start;
                noteBodyEl.selectionEnd = start + joined.length;
            }
            scheduleSave();
            textHistory.push(noteBodyEl.value);
            return;
        }

        // Bold, Italic, Link (Original logic)
        let prefix = '', suffix = '';
        if (e.key.toLowerCase() === 'b') {
            e.preventDefault();
            prefix = '**'; suffix = '**';
        } else if (e.key.toLowerCase() === 'i') {
            e.preventDefault();
            prefix = '*'; suffix = '*';
        } else if (e.key.toLowerCase() === 'k') {
            e.preventDefault();
            prefix = '['; suffix = '](url)';
        }
        
        if (prefix && suffix) {
            const selectedText = val.substring(start, end);
            
            noteBodyEl.value = val.substring(0, start) + prefix + selectedText + suffix + val.substring(end);
            
            if (e.key.toLowerCase() === 'k') {
                noteBodyEl.setSelectionRange(start + prefix.length + selectedText.length + 2, start + prefix.length + selectedText.length + 5);
            } else {
                noteBodyEl.setSelectionRange(start + prefix.length, end + prefix.length);
            }
            
            scheduleSave();
            textHistory.push(noteBodyEl.value);
        }
    }
});

// Delete note button (adding it dynamically to header later, or hotkey)
document.addEventListener('keydown', async (e) => {
    // Intercept Ctrl+Z / Cmd+Z for custom Undo/Redo
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z') {
        e.preventDefault();
        const isTextInput = document.activeElement && (document.activeElement.tagName === 'TEXTAREA' || document.activeElement.tagName === 'INPUT' || document.activeElement.isContentEditable);
        if (e.shiftKey) {
            if (isTextInput) document.getElementById('redo-btn').click();
            else {
                const sRedo = document.getElementById('sidebar-redo-btn');
                if (sRedo) sRedo.click();
            }
        } else {
            if (isTextInput) document.getElementById('undo-btn').click();
            else {
                const sUndo = document.getElementById('sidebar-undo-btn');
                if (sUndo) sUndo.click();
            }
        }
        return;
    }
    // Intercept Ctrl+Y / Cmd+Y for custom Redo
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'y') {
        e.preventDefault();
        const isTextInput = document.activeElement && (document.activeElement.tagName === 'TEXTAREA' || document.activeElement.tagName === 'INPUT' || document.activeElement.isContentEditable);
        if (isTextInput) document.getElementById('redo-btn').click();
        else {
            const sRedo = document.getElementById('sidebar-redo-btn');
            if (sRedo) sRedo.click();
        }
        return;
    }
    
    // Ctrl+Backspace or Cmd+Backspace to delete
    if ((e.ctrlKey || e.metaKey) && e.key === 'Backspace') {
        if (activeNoteId && await showConfirm('Delete this note?')) {
            try {
                const note = currentNotes.find(n => n.id === activeNoteId);
                const content = noteBodyEl.value; // Content of active note is in the editor
                await invoke('delete_note', { id: activeNoteId });
                currentNotes = currentNotes.filter(n => n.id !== activeNoteId);
                pushSidebarAction({ type: 'DELETE_NOTE', note: { ...note }, content: content });
                activeNoteId = null;
                
                if (currentNotes.length > 0) {
                    selectNote(currentNotes[0].id);
                } else {
                    noteTitleEl.value = '';
                    noteBodyEl.value = '';
                    renderNotes();
                }
            } catch (err) {
                console.error('Failed to delete', err);
            }
        }
    }
});

const newFolderBtn = document.getElementById('new-folder-btn');
if (newFolderBtn) {
    newFolderBtn.onclick = async () => {
        const name = await showPrompt("Enter folder name:", "New Folder");
        if (name && name.trim()) {
            currentFolder = name.trim();
            folderState[name.trim()] = true;
            newNoteBtn.click();
        }
    };
}

// Settings Modal Logic
const settingsModal = document.getElementById('settings-modal');
const openSettingsBtn = document.getElementById('open-settings-btn');
const closeSettingsBtn = document.getElementById('close-settings-btn');
const saveSettingsBtn = document.getElementById('save-settings-btn');

const themeSelect = document.getElementById('theme-select');
const fontSelect = document.getElementById('font-select');
const fontSizeSelect = document.getElementById('font-size-select');
const themeAccentColor = document.getElementById('theme-accent-color');
function applyPreferences(prefs) {
    document.documentElement.setAttribute('data-theme', prefs.theme || 'light');
    document.documentElement.style.setProperty('--accent-color', prefs.accentColor || '#ff5e00');
    document.documentElement.style.setProperty('--accent', prefs.accentColor || '#ff5e00');
    
    document.body.style.fontFamily = prefs.font || "'Inter', sans-serif";
    const fontSize = prefs.fontSize || "16px";
    document.documentElement.style.fontSize = fontSize;
    
    // Update inputs
    if (themeSelect) themeSelect.value = prefs.theme || 'light';
    if (fontSelect) fontSelect.value = prefs.font || "'Inter', sans-serif";
    if (fontSizeSelect) fontSizeSelect.value = fontSize;
    if (themeAccentColor) themeAccentColor.value = prefs.accentColor || '#ff5e00';
}

function loadPreferences() {
    const saved = localStorage.getItem('norm-preferences');
    if (saved) {
        applyPreferences(JSON.parse(saved));
    } else {
        applyPreferences({});
        // First install popup
        setTimeout(() => {
            if (openSettingsBtn) openSettingsBtn.click();
            showAlert("Welcome to NormNote!\n\nTo sync your notes across devices, please choose a Vault Location inside your iCloud Drive or Google Drive.");
        }, 500);
    }
}

const backupVaultBtn = document.getElementById('backup-vault-btn');
const restoreVaultBtn = document.getElementById('restore-vault-btn');

const vaultLocationInput = document.getElementById('vault-location-input');
const changeVaultBtn = document.getElementById('change-vault-btn');

async function updateVaultLocationUI() {
    if (invoke && vaultLocationInput) {
        try {
            const path = await invoke('get_current_vault_path');
            vaultLocationInput.value = path;
        } catch (e) {
            console.error("Failed to get vault path", e);
        }
    }
}

if (openSettingsBtn) openSettingsBtn.onclick = () => {
    updateVaultLocationUI();
    settingsModal.style.display = 'flex';
};
if (closeSettingsBtn) closeSettingsBtn.onclick = () => settingsModal.style.display = 'none';

if (changeVaultBtn) {
    changeVaultBtn.onclick = async () => {
        try {
            const newPath = await invoke('choose_vault_location_dialog');
            if (newPath) {
                await invoke('set_vault_location', { path: newPath });
                vaultLocationInput.value = newPath;
                showAlert('Vault location updated successfully!');
                await loadNotes(); // Reload notes from new vault
            }
        } catch (e) {
            if (e !== 'Cancelled') {
                console.error("Failed to change vault location", e);
                showAlert('Failed to change vault location: ' + e);
            }
        }
    };
}

if (backupVaultBtn) {
    backupVaultBtn.onclick = async () => {
        try {
            const path = await invoke('backup_vault');
            showAlert(`Vault backed up successfully to:\n${path}`);
        } catch (e) {
            if (e !== 'Cancelled') {
                showAlert('Backup failed: ' + e);
            }
        }
    };
}

if (restoreVaultBtn) {
    restoreVaultBtn.onclick = async () => {
        if (await showConfirm("Are you sure you want to restore? This will overwrite your current vault.")) {
            try {
                await invoke('restore_vault');
                showAlert('Vault restored successfully!');
                await loadNotes(); // Reload notes
            } catch (e) {
                if (e !== 'Cancelled') {
                    showAlert('Restore failed: ' + e);
                }
            }
        }
    };
}

function savePreferences() {
    const prefs = {
        theme: themeSelect.value,
        font: fontSelect.value,
        fontSize: fontSizeSelect.value,
        accentColor: themeAccentColor.value
    };
    localStorage.setItem('norm-preferences', JSON.stringify(prefs));
    applyPreferences(prefs);
}

if (themeSelect) themeSelect.onchange = savePreferences;
if (fontSelect) fontSelect.onchange = savePreferences;
if (fontSizeSelect) fontSizeSelect.onchange = savePreferences;
if (themeAccentColor) themeAccentColor.oninput = savePreferences;
// Removed save button

// Init
window.addEventListener('DOMContentLoaded', () => {
    loadPreferences();
    
    // Basic Tauri check
    if (window.__TAURI__) {
        loadNotes();
    } else {
        console.error("Not running in Tauri environment");
    }
    
    // Sidebar Resizer Logic
    const resizer = document.getElementById('sidebar-resizer');
    const sidebar = document.querySelector('.sidebar');
    if (resizer && sidebar) {
        let isResizing = false;
        
        // Restore saved width
        const savedWidth = localStorage.getItem('sidebar-width');
        if (savedWidth) {
            document.documentElement.style.setProperty('--sidebar-width', savedWidth + 'px');
        }

        resizer.addEventListener('mousedown', (e) => {
            isResizing = true;
            document.body.style.cursor = 'col-resize';
            sidebar.classList.add('is-resizing');
            resizer.classList.add('is-resizing');
        });

        document.addEventListener('mousemove', (e) => {
            if (!isResizing) return;
            const newWidth = e.clientX;
            document.documentElement.style.setProperty('--sidebar-width', newWidth + 'px');
        });

        document.addEventListener('mouseup', () => {
            if (isResizing) {
                isResizing = false;
                document.body.style.cursor = '';
                sidebar.classList.remove('is-resizing');
                resizer.classList.remove('is-resizing');
                
                // Save width
                const finalWidth = sidebar.getBoundingClientRect().width;
                localStorage.setItem('sidebar-width', finalWidth);
            }
        });
    }
});

// Markdown Preview Logic
let isPreviewMode = false;
const modeToggleBtn = document.getElementById('mode-toggle');
const modeEditBtn = document.getElementById('mode-edit-btn');
const modeViewBtn = document.getElementById('mode-view-btn');

const previewContainer = document.createElement('div');
previewContainer.className = 'markdown-preview body-input';
previewContainer.style.display = 'none';
noteBodyEl.parentNode.insertBefore(previewContainer, noteBodyEl.nextSibling);

async function setPreviewMode(isView) {
    isPreviewMode = isView;
    if (isPreviewMode) {
        modeToggleBtn.classList.add('view');
        
        const mdConverter = window.showdown ? new showdown.Converter({ 
            tables: true, 
            strikethrough: true, 
            tasklists: true,
            ghCodeBlocks: true,
            simpleLineBreaks: true,
            requireSpaceBeforeHeadingText: true 
        }) : null;
        
        if (mdConverter) {
            previewContainer.innerHTML = mdConverter.makeHtml(noteBodyEl.value);
            
            // Interactive Checkboxes
            const checkboxes = previewContainer.querySelectorAll('input[type="checkbox"]');
            checkboxes.forEach((cb, index) => {
                cb.removeAttribute('disabled');
                cb.style.cursor = 'pointer';
                cb.addEventListener('change', async (e) => {
                    const isChecked = e.target.checked;
                    let text = noteBodyEl.value;
                    let count = -1;
                    
                    const regex = /^(\s*[-*+]\s+)\[([ xX])\]/gm;
                    let newText = text.replace(regex, (match, prefix, state) => {
                        count++;
                        if (count === index) {
                            return prefix + (isChecked ? '[x]' : '[ ]');
                        }
                        return match;
                    });
                    
                    noteBodyEl.value = newText;
                    if (activeNoteId) {
                        textHistory.push(newText);
                        await invoke('save_note', { id: activeNoteId, content: newText });
                        renderNotes();
                    }
                });
            });
            
            // Resolve local image paths
            const imgs = previewContainer.querySelectorAll('img');
            for (let img of imgs) {
                const src = img.getAttribute('src');
                if (src && src.startsWith('.assets/')) {
                    try {
                        const bytes = await invoke('read_image_bytes', { path: src });
                        const blob = new Blob([new Uint8Array(bytes)]);
                        const url = URL.createObjectURL(blob);
                        img.src = url;
                    } catch (e) {
                        console.error("Failed to load image", src, e);
                    }
                }
            }
        } else {
            previewContainer.innerHTML = 'Markdown preview not loaded';
        }
        
        noteBodyEl.style.display = 'none';
        previewContainer.style.display = 'block';
    } else {
        modeToggleBtn.classList.remove('view');
        
        noteBodyEl.style.display = 'block';
        previewContainer.style.display = 'none';
        noteBodyEl.focus();
    }
}

if (modeToggleBtn) modeToggleBtn.onclick = () => {
    setPreviewMode(!isPreviewMode);
};
if (modeEditBtn) modeEditBtn.onclick = (e) => {
    e.stopPropagation();
    setPreviewMode(false);
};
if (modeViewBtn) modeViewBtn.onclick = (e) => {
    e.stopPropagation();
    setPreviewMode(true);
};

// Export to PDF
document.getElementById('export-pdf-btn')?.addEventListener('click', () => {
    if (!activeNoteId) {
        showAlert("Please open a note to export.");
        return;
    }
    
    // Switch to preview mode temporarily if not already
    const wasPreview = isPreviewMode;
    if (!wasPreview) setPreviewMode(true);
    
    setTimeout(() => {
        const element = document.createElement('div');
        element.style.padding = '20px';
        element.style.fontFamily = 'Inter, sans-serif';
        element.innerHTML = previewContainer.innerHTML;
        
        const noteName = activeNoteId.split('/').pop().replace('.md', '');
        
        const opt = {
            margin:       10,
            filename:     `${noteName}.pdf`,
            image:        { type: 'jpeg', quality: 0.98 },
            html2canvas:  { scale: 2 },
            jsPDF:        { unit: 'mm', format: 'a4', orientation: 'portrait' }
        };
        
        html2pdf().set(opt).from(element).save().then(() => {
            if (!wasPreview) setPreviewMode(false);
        });
    }, 100);
});

// Undo/Redo Logic
const undoBtnEl = document.getElementById('undo-btn');
if (undoBtnEl) undoBtnEl.onclick = () => {
    if (document.activeElement === noteBodyEl || document.activeElement === noteTitleEl) {
        const prevState = textHistory.undo();
        if (prevState !== null) {
            noteBodyEl.value = prevState;
            scheduleSave();
        }
        noteBodyEl.focus();
    } else {
        document.getElementById('sidebar-undo-btn')?.click();
    }
};

const redoBtnEl = document.getElementById('redo-btn');
if (redoBtnEl) redoBtnEl.onclick = () => {
    if (document.activeElement === noteBodyEl || document.activeElement === noteTitleEl) {
        const nextState = textHistory.redo();
        if (nextState !== null) {
            noteBodyEl.value = nextState;
            scheduleSave();
        }
        noteBodyEl.focus();
    } else {
        document.getElementById('sidebar-redo-btn')?.click();
    }
};

// Export Logic
const exportBtn = document.getElementById('export-btn');
const exportDropdown = document.getElementById('export-dropdown');

if (exportBtn && exportDropdown) {
    exportBtn.onclick = (e) => {
        e.stopPropagation();
        exportDropdown.style.display = exportDropdown.style.display === 'none' ? 'block' : 'none';
        
        // Hide import dropdown if open
        const impDropdown = document.getElementById('import-dropdown');
        if (impDropdown) impDropdown.style.display = 'none';
    };

    document.addEventListener('click', (e) => {
        if (exportDropdown.style.display === 'block' && !exportDropdown.contains(e.target) && e.target !== exportBtn) {
            exportDropdown.style.display = 'none';
        }
    });

    document.querySelectorAll('#export-dropdown .dropdown-item').forEach(item => {
        item.onclick = async (e) => {
            e.stopPropagation();
            exportDropdown.style.display = 'none';
            
            if (!activeNoteId) return;
            
            const format = item.dataset.format;
            const title = noteTitleEl.value.trim() || 'Untitled Note';
            const body = noteBodyEl.value;
            
            const md_content = `# ${title}\n\n${body}`;
            const txt_content = `${title}\n\n${body}`;
            
            const mdConverter = new showdown.Converter({ 
                tables: true, strikethrough: true, tasklists: true,
                ghCodeBlocks: true, simpleLineBreaks: true, requireSpaceBeforeHeadingText: true 
            });
            const html_content = `<!DOCTYPE html><html><head><title>${title}</title></head><body><h1>${title}</h1>\n${window.showdown ? mdConverter.makeHtml(body) : body}</body></html>`;
            
            if (format === 'pdf') {
                const previewContainer = document.querySelector('.markdown-preview');
                if (previewContainer && window.showdown) {
                    let titleHtml = '';
                    const lines = body.split('\n');
                    if (lines.length > 0 && lines[0].startsWith('# ')) {
                        // Title in body
                    } else if (title) {
                        titleHtml = `<h1>${title}</h1>\n`;
                    }
                    const el = document.createElement('div');
                    el.innerHTML = titleHtml + mdConverter.makeHtml(body);
                    el.style.padding = '20px';
                    el.style.fontFamily = document.body.style.fontFamily;
                    el.style.fontSize = document.documentElement.style.fontSize;
                    el.style.color = 'black'; // force print colors
                    
                    if (window.html2pdf) {
                        const opt = {
                            margin:       0.5,
                            filename:     `${title.replace(/[/\\?%*:|"<>]/g, '-')}.pdf`,
                            image:        { type: 'jpeg', quality: 0.98 },
                            html2canvas:  { scale: 2 },
                            jsPDF:        { unit: 'in', format: 'a4', orientation: 'portrait' }
                        };
                        html2pdf().set(opt).from(el).outputPdf('datauristring').then(async base64 => {
                            try {
                                await invoke('export_pdf_dialog', {
                                    title: opt.filename,
                                    base64Data: base64
                                });
                                showAlert('Done! Export successfully completed.');
                            } catch (err) {
                                console.error('PDF export failed', err);
                                showAlert('PDF Export failed: ' + err);
                            }
                        }).catch(err => {
                            console.error('PDF generation failed', err);
                            showAlert('PDF Generation failed: ' + err);
                        });
                    } else {
                        showAlert('PDF Exporter is loading, please try again in a moment.');
                    }
                }
                return;
            }
            
            try {
                await invoke('export_note_dialog', { 
                    format: format,
                    title: title.replace(/[/\\?%*:|"<>]/g, '-'),
                    mdContent: md_content,
                    txtContent: txt_content,
                    htmlContent: html_content
                });
                showAlert('Done! Export successfully completed.');
            } catch (err) {
                console.error('Export failed', err);
                showAlert('Export failed: ' + err);
            }
        };
    });
}


// Custom Modal Logic
function updateTagsUI() {
    const tagsContainer = document.getElementById('tags-list');
    if (tagsContainer) {
        if (tagsContainer) tagsContainer.innerHTML = '';
        const allTags = new Set();
        currentNotes.forEach(n => {
            if (n.tags) n.tags.forEach(t => allTags.add(t));
        });
        
        // Add "All" tag
        const allBtn = document.createElement('button');
        allBtn.textContent = 'All';
        allBtn.className = 'icon-btn';
        allBtn.style.padding = '4px 12px';
        allBtn.style.borderRadius = '12px';
        allBtn.style.fontSize = '12px';
        allBtn.style.flexShrink = '0';
        allBtn.style.whiteSpace = 'nowrap';
        allBtn.style.width = 'auto';
        allBtn.style.height = 'auto';
        allBtn.style.background = activeTag === null ? 'var(--accent)' : 'transparent';
        allBtn.style.color = activeTag === null ? 'white' : 'inherit';
        allBtn.style.border = '1px solid var(--border)';
        allBtn.onclick = () => {
            activeTag = null;
            loadNotes();
        };
        tagsContainer.appendChild(allBtn);
        
        // Add other tags
        Array.from(allTags).sort().forEach(tag => {
            const btn = document.createElement('button');
            btn.textContent = '#' + tag;
            btn.className = 'icon-btn';
            btn.style.padding = '4px 12px';
            btn.style.borderRadius = '12px';
            btn.style.fontSize = '12px';
            btn.style.flexShrink = '0';
            btn.style.whiteSpace = 'nowrap';
            btn.style.width = 'auto';
            btn.style.height = 'auto';
            btn.style.background = activeTag === tag ? 'var(--accent)' : 'transparent';
            btn.style.color = activeTag === tag ? 'white' : 'inherit';
            btn.style.border = '1px solid var(--border)';
            btn.onclick = () => {
                activeTag = tag;
                loadNotes();
            };
            tagsContainer.appendChild(btn);
        });
    }
}

function showPrompt(message, defaultValue = "") {
    return new Promise((resolve) => {
        const overlay = document.getElementById('custom-modal-overlay');
        const msgEl = document.getElementById('modal-message');
        const inputEl = document.getElementById('modal-input');
        const okBtn = document.getElementById('modal-ok-btn');
        const cancelBtn = document.getElementById('modal-cancel-btn');
        
        msgEl.textContent = message;
        inputEl.style.display = 'block';
        inputEl.value = defaultValue;
        overlay.style.display = 'flex';
        inputEl.focus();
        inputEl.select();
        
        const cleanup = () => {
            overlay.style.display = 'none';
            okBtn.onclick = null;
            cancelBtn.onclick = null;
            inputEl.onkeydown = null;
        };
        
        okBtn.onclick = () => {
            cleanup();
            resolve(inputEl.value);
        };
        
        cancelBtn.onclick = () => {
            cleanup();
            resolve(null);
        };
        
        inputEl.onkeydown = (e) => {
            if (e.key === 'Enter') {
                cleanup();
                resolve(inputEl.value);
            } else if (e.key === 'Escape') {
                cleanup();
                resolve(null);
            }
        };
    });
}

function showConfirm(message) {
    return new Promise((resolve) => {
        const overlay = document.getElementById('custom-modal-overlay');
        const msgEl = document.getElementById('modal-message');
        const inputEl = document.getElementById('modal-input');
        const okBtn = document.getElementById('modal-ok-btn');
        const cancelBtn = document.getElementById('modal-cancel-btn');
        
        msgEl.textContent = message;
        inputEl.style.display = 'none';
        overlay.style.display = 'flex';
        okBtn.focus();
        
        const cleanup = () => {
            overlay.style.display = 'none';
            okBtn.onclick = null;
            cancelBtn.onclick = null;
        };
        
        okBtn.onclick = () => {
            cleanup();
            resolve(true);
        };
        
        cancelBtn.onclick = () => {
            cleanup();
            resolve(false);
        };
    });
}

// --- Sidebar History (Undo/Redo) ---
const sidebarHistory = [];
let sidebarHistoryIndex = -1;

function pushSidebarAction(action) {
    // action: { type: 'RENAME_NOTE', oldId, newId, content }
    // action: { type: 'MOVE_NOTE', noteId, oldFolder, newFolder }
    // action: { type: 'DELETE_NOTE', note }
    
    // Discard any future history
    sidebarHistory.splice(sidebarHistoryIndex + 1);
    sidebarHistory.push(action);
    sidebarHistoryIndex++;
    updateSidebarHistoryButtons();
}

function updateSidebarHistoryButtons() {
    const undoBtn = document.getElementById('sidebar-undo-btn');
    const redoBtn = document.getElementById('sidebar-redo-btn');
    if (undoBtn) undoBtn.style.opacity = sidebarHistoryIndex >= 0 ? '1' : '0.3';
    if (redoBtn) redoBtn.style.opacity = sidebarHistoryIndex < sidebarHistory.length - 1 ? '1' : '0.3';
}

const sidebarUndoBtn = document.getElementById('sidebar-undo-btn');
const sidebarRedoBtn = document.getElementById('sidebar-redo-btn');

if (sidebarUndoBtn) sidebarUndoBtn.onclick = async () => {
    if (sidebarHistoryIndex < 0) return;
    const action = sidebarHistory[sidebarHistoryIndex];
    sidebarHistoryIndex--;
    updateSidebarHistoryButtons();
    
    if (action.type === 'MOVE_NOTE') {
        const { noteId, oldFolder, newFolder } = action;
        const note = currentNotes.find(n => n.id === noteId);
        if (note) {
            const filename = note.id.split('/').pop();
            const originalId = oldFolder === '/' ? filename : (oldFolder ? oldFolder + '/' + filename : filename);
            try {
                const content = await invoke('read_note', { id: noteId });
                await invoke('save_note', { id: originalId, content: content });
                await invoke('delete_note', { id: noteId });
                note.id = originalId;
                if (activeNoteId === noteId) activeNoteId = originalId;
                renderNotes();
            } catch (e) { console.error(e); }
        }
    } else if (action.type === 'RENAME_FOLDER') {
        const { oldFolder, newFolder } = action;
        await renameFolder(newFolder, oldFolder, false); // false = don't push to history
    } else if (action.type === 'RENAME_NOTE') {
        const { oldId, newId } = action;
        try {
            const content = await invoke('read_note', { id: newId });
            await invoke('save_note', { id: oldId, content: content });
            await invoke('delete_note', { id: newId });
            const note = currentNotes.find(n => n.id === newId);
            if (note) {
                note.id = oldId;
                const parts = content.split('\n');
                note.title = parts[0] ? parts[0].replace(/^# /, '').trim() : 'Untitled Note';
                if (activeNoteId === newId) activeNoteId = oldId;
                
                const idx = noteOrder.indexOf(newId);
                if (idx !== -1) {
                    noteOrder[idx] = oldId;
                    saveNoteOrder();
                }
            }
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'MERGE_NOTE') {
        const { srcId, dstId, srcContent, dstContent, srcNoteObj } = action;
        try {
            await invoke('save_note', { id: srcId, content: srcContent });
            await invoke('save_note', { id: dstId, content: dstContent });
            
            if (srcNoteObj) currentNotes.push(srcNoteObj);
            
            const dstNote = currentNotes.find(n => n.id === dstId);
            if (dstNote) {
                const parts = dstContent.split('\n\n');
                dstNote.title = parts[0] ? parts[0].replace(/^# /, '') : 'Untitled Note';
                dstNote.preview = parts.slice(1).join('\n').substring(0, 50).replace(/\n/g, ' ');
            }
            
            if (activeNoteId === dstId) {
                const parts = dstContent.split('\n\n');
                noteTitleEl.value = parts[0] ? parts[0].replace(/^# /, '') : '';
                const body = parts.slice(1).join('\n\n');
                noteBodyEl.value = body;
                textHistory.reset(body);
            }
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'DUPLICATE_NOTE') {
        const { newId } = action;
        try {
            await invoke('delete_note', { id: newId });
            currentNotes = currentNotes.filter(n => n.id !== newId);
            noteOrder = noteOrder.filter(id => id !== newId);
            saveNoteOrder();
            if (activeNoteId === newId) activeNoteId = null;
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'REORDER') {
        noteOrder = [...action.oldOrder];
        saveNoteOrder();
        renderNotes();
    } else if (action.type === 'DUPLICATE_FOLDER') {
        const { newFolder } = action;
        const notesToDel = currentNotes.filter(n => n.id.startsWith(newFolder + '/'));
        for (const n of notesToDel) {
            try {
                await invoke('delete_note', { id: n.id });
            } catch(e) {}
        }
        currentNotes = currentNotes.filter(n => !n.id.startsWith(newFolder + '/'));
        if (activeNoteId && activeNoteId.startsWith(newFolder + '/')) activeNoteId = null;
        renderNotes();
    } else if (action.type === 'DELETE_NOTE') {
        const { note, content } = action;
        try {
            await invoke('save_note', { id: note.id, content: content });
            currentNotes.push(note);
            if (!noteOrder.includes(note.id)) {
                noteOrder.push(note.id);
                saveNoteOrder();
            }
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'BATCH_DELETE') {
        const { notes } = action;
        await Promise.all(notes.map(async (item) => {
            try {
                await invoke('save_note', { id: item.note.id, content: item.content });
                currentNotes.push(item.note);
                if (!noteOrder.includes(item.note.id)) noteOrder.push(item.note.id);
            } catch(e) {}
        }));
        await invoke('flush_workspace');
        saveNoteOrder();
        renderNotes();
    } else if (action.type === 'BATCH_MOVE') {
        const { moves } = action;
        for (const m of moves) {
            await moveNoteToFolder(m.noteId, m.oldFolder, false);
        }
        renderNotes();
    } else if (action.type === 'BATCH_MERGE') {
        const { targetId, oldTargetContent, deletedNotes } = action;
        try {
            await invoke('save_note', { id: targetId, content: oldTargetContent });
            const targetNote = currentNotes.find(n => n.id === targetId);
            if (targetNote) {
                const parts = oldTargetContent.split('\n\n');
                targetNote.title = parts[0] ? parts[0].replace(/^# /, '') : 'Untitled Note';
                targetNote.preview = parts.slice(1).join('\n').substring(0, 50).replace(/\n/g, ' ');
            }
            await Promise.all(deletedNotes.map(async (d) => {
                await invoke('save_note', { id: d.note.id, content: d.content });
                currentNotes.push(d.note);
                if (!noteOrder.includes(d.note.id)) noteOrder.push(d.note.id);
            }));
            await invoke('flush_workspace');
            saveNoteOrder();
            if (activeNoteId === targetId) {
                const parts = oldTargetContent.split('\n\n');
                noteTitleEl.value = parts[0] ? parts[0].replace(/^# /, '') : '';
                const body = parts.slice(1).join('\n\n');
                noteBodyEl.value = body;
            }
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'DELETE_FOLDER') {
        const { folder, notes } = action;
        try {
            await Promise.all(notes.map(async ({ note, content }) => {
                await invoke('save_note', { id: note.id, content: content });
                currentNotes.push(note);
                if (!noteOrder.includes(note.id)) noteOrder.push(note.id);
            }));
            await invoke('flush_workspace');
            saveNoteOrder();
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'BATCH_DUPLICATE') {
        const { notes } = action;
        await Promise.all(notes.map(async (item) => {
            try {
                await invoke('delete_note', { id: item.note.id });
                currentNotes = currentNotes.filter(n => n.id !== item.note.id);
                noteOrder = noteOrder.filter(id => id !== item.note.id);
                if (activeNoteId === item.note.id) activeNoteId = null;
            } catch(e) {}
        }));
        await invoke('flush_workspace');
        saveNoteOrder();
        if (!activeNoteId && currentNotes.length > 0) selectNote(currentNotes[0].id);
        else if (!activeNoteId) { noteTitleEl.value = ''; noteBodyEl.value = ''; }
        renderNotes();
    }
};

if (sidebarRedoBtn) sidebarRedoBtn.onclick = async () => {
    if (sidebarHistoryIndex >= sidebarHistory.length - 1) return;
    sidebarHistoryIndex++;
    const action = sidebarHistory[sidebarHistoryIndex];
    updateSidebarHistoryButtons();
    
    if (action.type === 'MOVE_NOTE') {
        const { noteId, oldFolder, newFolder } = action;
        // The current ID is now the old original ID because we undid it
        const filename = noteId.split('/').pop();
        const originalId = oldFolder === '/' ? filename : (oldFolder ? oldFolder + '/' + filename : filename);
        
        const note = currentNotes.find(n => n.id === originalId);
        if (note) {
            try {
                const content = await invoke('read_note', { id: originalId });
                await invoke('save_note', { id: noteId, content: content });
                await invoke('delete_note', { id: originalId });
                note.id = noteId;
                if (activeNoteId === originalId) activeNoteId = noteId;
                renderNotes();
            } catch(e) {}
        }
    } else if (action.type === 'RENAME_FOLDER') {
        const { oldFolder, newFolder } = action;
        await renameFolder(oldFolder, newFolder, false);
    } else if (action.type === 'RENAME_NOTE') {
        const { oldId, newId } = action;
        try {
            const content = await invoke('read_note', { id: oldId });
            await invoke('save_note', { id: newId, content: content });
            await invoke('delete_note', { id: oldId });
            const note = currentNotes.find(n => n.id === oldId);
            if (note) {
                note.id = newId;
                const parts = content.split('\n');
                note.title = parts[0] ? parts[0].replace(/^# /, '').trim() : 'Untitled Note';
                if (activeNoteId === oldId) activeNoteId = newId;
                
                const idx = noteOrder.indexOf(oldId);
                if (idx !== -1) {
                    noteOrder[idx] = newId;
                    saveNoteOrder();
                }
            }
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'MERGE_NOTE') {
        const { srcId, dstId, srcContent, dstContent } = action;
        try {
            const lines = srcContent.split('\n');
            if (lines.length > 0 && lines[0].startsWith('#')) lines.shift();
            const mergedContent = dstContent + '\n\n---\n\n' + lines.join('\n').trim();
            
            await invoke('save_note', { id: dstId, content: mergedContent });
            await invoke('delete_note', { id: srcId });
            
            currentNotes = currentNotes.filter(n => n.id !== srcId);
            
            const dstNote = currentNotes.find(n => n.id === dstId);
            if (dstNote) {
                const parts = mergedContent.split('\n\n');
                dstNote.title = parts[0] ? parts[0].replace(/^# /, '') : 'Untitled Note';
                dstNote.preview = parts.slice(1).join('\n').substring(0, 50).replace(/\n/g, ' ');
            }
            
            if (activeNoteId === srcId) activeNoteId = null;
            if (activeNoteId === dstId) {
                const parts = mergedContent.split('\n\n');
                noteTitleEl.value = parts[0] ? parts[0].replace(/^# /, '') : '';
                const body = parts.slice(1).join('\n\n');
                noteBodyEl.value = body;
                textHistory.reset(body);
            }
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'DUPLICATE_NOTE') {
        const { oldId, newId } = action;
        try {
            const content = await invoke('read_note', { id: oldId });
            await invoke('save_note', { id: newId, content: content });
            const parts = content.split('\n');
            const title = parts[0] ? parts[0].replace(/^# /, '').trim() : 'Untitled Note';
            const note = { id: newId, updated: Date.now(), title: title + ' (copy)', preview: '' };
            currentNotes.push(note);
            if (!noteOrder.includes(newId)) {
                noteOrder.push(newId);
                saveNoteOrder();
            }
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'REORDER') {
        noteOrder = [...action.newOrder];
        saveNoteOrder();
        renderNotes();
    } else if (action.type === 'DUPLICATE_FOLDER') {
        const { oldFolder, newFolder } = action;
        try {
            for (const note of currentNotes.filter(n => n.id.startsWith(oldFolder + '/'))) {
                const filename = note.id.split('/').pop();
                const newId = newFolder + '/' + filename;
                const content = await invoke('read_note', { id: note.id });
                await invoke('save_note', { id: newId, content: content });
                currentNotes.push({ id: newId, updated: Date.now(), title: note.title, preview: note.preview });
                if (!noteOrder.includes(newId)) noteOrder.push(newId);
            }
            saveNoteOrder();
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'DELETE_NOTE') {
        const { note } = action;
        try {
            await invoke('delete_note', { id: note.id });
            currentNotes = currentNotes.filter(n => n.id !== note.id);
            noteOrder = noteOrder.filter(id => id !== note.id);
            saveNoteOrder();
            if (activeNoteId === note.id) activeNoteId = null;
            if (!activeNoteId && currentNotes.length > 0) selectNote(currentNotes[0].id);
            else if (!activeNoteId) { noteTitleEl.value = ''; noteBodyEl.value = ''; }
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'BATCH_DELETE') {
        const { notes } = action;
        await Promise.all(notes.map(async (item) => {
            try {
                await invoke('delete_note', { id: item.note.id });
                currentNotes = currentNotes.filter(n => n.id !== item.note.id);
                noteOrder = noteOrder.filter(id => id !== item.note.id);
                if (activeNoteId === item.note.id) activeNoteId = null;
            } catch(e) {}
        }));
        await invoke('flush_workspace');
        saveNoteOrder();
        if (!activeNoteId && currentNotes.length > 0) selectNote(currentNotes[0].id);
        else if (!activeNoteId) { noteTitleEl.value = ''; noteBodyEl.value = ''; }
        renderNotes();
    } else if (action.type === 'BATCH_MOVE') {
        const { moves } = action;
        for (const m of moves) {
            const currentId = m.oldFolder === '/' ? m.noteId.split('/').pop() : m.oldFolder + '/' + m.noteId.split('/').pop();
            await moveNoteToFolder(currentId, m.newFolder, false);
        }
        renderNotes();
    } else if (action.type === 'BATCH_MERGE') {
        const { targetId, newTargetContent, deletedNotes } = action;
        try {
            await invoke('save_note', { id: targetId, content: newTargetContent });
            const targetNote = currentNotes.find(n => n.id === targetId);
            if (targetNote) {
                const parts = newTargetContent.split('\n\n');
                targetNote.title = parts[0] ? parts[0].replace(/^# /, '') : 'Untitled Note';
                targetNote.preview = parts.slice(1).join('\n').substring(0, 50).replace(/\n/g, ' ');
            }
            await Promise.all(deletedNotes.map(async (d) => {
                await invoke('delete_note', { id: d.note.id });
                currentNotes = currentNotes.filter(n => n.id !== d.note.id);
                noteOrder = noteOrder.filter(id => id !== d.note.id);
                if (activeNoteId === d.note.id) activeNoteId = null;
            }));
            await invoke('flush_workspace');
            saveNoteOrder();
            if (activeNoteId === targetId) {
                const parts = newTargetContent.split('\n\n');
                noteTitleEl.value = parts[0] ? parts[0].replace(/^# /, '') : '';
                const body = parts.slice(1).join('\n\n');
                noteBodyEl.value = body;
            } else if (!activeNoteId && currentNotes.length > 0) {
                selectNote(currentNotes[0].id);
            }
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'DELETE_FOLDER') {
        const { folder, notes } = action;
        try {
            await Promise.all(notes.map(async ({ note }) => {
                await invoke('delete_note', { id: note.id });
                currentNotes = currentNotes.filter(n => n.id !== note.id);
                noteOrder = noteOrder.filter(id => id !== note.id);
                if (activeNoteId === note.id) activeNoteId = null;
            }));
            await invoke('flush_workspace');
            saveNoteOrder();
            renderNotes();
        } catch(e) {}
    } else if (action.type === 'BATCH_DUPLICATE') {
        const { notes } = action;
        await Promise.all(notes.map(async (item) => {
            try {
                await invoke('save_note', { id: item.note.id, content: item.content });
                currentNotes.push(item.note);
                if (!noteOrder.includes(item.note.id)) noteOrder.push(item.note.id);
            } catch(e) {}
        }));
        await invoke('flush_workspace');
        saveNoteOrder();
        renderNotes();
    }
};

// --- Custom Reordering Logic ---
let noteOrder = [];
try {
    const savedOrder = localStorage.getItem('norm-note-order');
    if (savedOrder) noteOrder = JSON.parse(savedOrder);
} catch (e) {}

function saveNoteOrder() {
    localStorage.setItem('norm-note-order', JSON.stringify(noteOrder));
}

async function handleSidebarDrop(targetId, draggedRawData, position) {
    // position: 'before' or 'after'
    if (!draggedRawData.startsWith('NOTE::') && !draggedRawData.startsWith('FOLDER::') && !draggedRawData.startsWith('MULTINOTE::')) return;
    
    const isMultiNote = draggedRawData.startsWith('MULTINOTE::');
    const isNote = draggedRawData.startsWith('NOTE::') || isMultiNote;
    
    let draggedIds = [];
    if (isMultiNote) {
        draggedIds = JSON.parse(draggedRawData.substring(11));
    } else {
        draggedIds = [isNote ? draggedRawData.substring(6) : 'FOLDER::' + draggedRawData.substring(8)];
    }
    
    const oldOrder = [...noteOrder];
    const batchMoves = [];
    
    // Check if we are dragging a note into a different folder's note list
    if (isNote && !targetId.startsWith('FOLDER::')) {
        const targetFolder = targetId.includes('/') ? targetId.substring(0, targetId.lastIndexOf('/')) : '';
        
        for (let i = 0; i < draggedIds.length; i++) {
            let draggedId = draggedIds[i];
            const draggedFolder = draggedId.includes('/') ? draggedId.substring(0, draggedId.lastIndexOf('/')) : '';
            
            if (targetFolder !== draggedFolder) {
                const res = await moveNoteToFolder(draggedId, targetFolder, false);
                if (res) {
                    batchMoves.push(res);
                    draggedIds[i] = res.noteId;
                }
            }
        }
    }
    
    let currentNoteOrder = [...noteOrder];
    for (const id of draggedIds) {
        currentNoteOrder = currentNoteOrder.filter(nId => nId !== id);
    }
    
    let targetIndex = currentNoteOrder.indexOf(targetId);
    if (targetIndex === -1) {
        currentNoteOrder.push(targetId);
        targetIndex = currentNoteOrder.length - 1;
    }
    
    if (position === 'before') {
        currentNoteOrder.splice(targetIndex, 0, ...draggedIds);
    } else {
        currentNoteOrder.splice(targetIndex + 1, 0, ...draggedIds);
    }
    
    noteOrder = currentNoteOrder;
    saveNoteOrder();
    
    pushSidebarAction({ type: 'REORDER', oldOrder: oldOrder, newOrder: [...noteOrder] });
    
    if (batchMoves.length > 0) {
        pushSidebarAction({ type: 'BATCH_MOVE', moves: batchMoves });
    }
    
    if (isMultiNote) {
        selectedNoteIds.clear();
        updateBulkActionBar();
    }
    
    renderNotes();
}
// Global state for internal drag and drop to bypass WebView2 restrictions
window.draggedItemRawData = null;

async function handleImageImport(path) {
    if (!activeNoteId || !path) return;
    try {
        const ext = path.split('.').pop().toLowerCase();
        if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg'].includes(ext)) {
            const relativePath = await invoke('import_image_asset', { filePath: path });
            insertImageToEditor(relativePath);
        } else {
            showAlert("Unsupported image type: " + ext);
        }
    } catch (e) {
        showAlert("Failed to import image: " + e);
        console.error("Failed to import image", e);
    }
}

async function handleImageBytes(file) {
    if (!activeNoteId || !file) return;
    try {
        const ext = file.name ? file.name.split('.').pop().toLowerCase() : "png";
        if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg'].includes(ext)) {
            const arrayBuffer = await file.arrayBuffer();
            const bytes = Array.from(new Uint8Array(arrayBuffer));
            const relativePath = await invoke('import_image_bytes', { bytes: bytes, ext: ext });
            insertImageToEditor(relativePath);
        } else {
            showAlert("Unsupported image type: " + ext);
        }
    } catch (e) {
        showAlert("Failed to import image bytes: " + e);
        console.error("Failed to import image bytes", e);
    }
}

function insertImageToEditor(relativePath) {
    const start = noteBodyEl.selectionStart;
    const val = noteBodyEl.value;
    const imgSyntax = `\n![image](${relativePath})\n`;
    
    noteBodyEl.value = val.substring(0, start) + imgSyntax + val.substring(noteBodyEl.selectionEnd);
    
    textHistory.push(noteBodyEl.value);
    scheduleSave();
    
    noteBodyEl.focus();
    noteBodyEl.setSelectionRange(start + imgSyntax.length, start + imgSyntax.length);
    
    if (isPreviewMode) setPreviewMode(true);
}

// Drag & Drop Images on Editor
noteBodyEl.addEventListener('dragover', (e) => {
    e.preventDefault();
});

noteBodyEl.addEventListener('drop', async (e) => {
    e.preventDefault();
    if (!activeNoteId) return;
    
    if (e.dataTransfer.files && e.dataTransfer.files.length > 0) {
        for (let file of e.dataTransfer.files) {
            await handleImageBytes(file);
        }
    }
});

// Insert Image Button
const insertImageBtn = document.getElementById('insert-image-btn');
if (insertImageBtn) {
    insertImageBtn.onclick = async () => {
        if (!activeNoteId) return;
        try {
            const path = await invoke('import_image_dialog');
            insertImageToEditor(path);
        } catch (e) {
            if (e !== "No file selected") {
                showAlert("Error in import_image_dialog: " + e);
                console.error(e);
            }
        }
    };
}

// Quick Capture
if (listen) {
    listen('quick-capture', () => {
        document.getElementById('new-note-btn').click();
        if (isPreviewMode) setPreviewMode(false);
        noteBodyEl.focus();
    });
}

// Search Bar Logic
const searchInput = document.getElementById('search-input');
if (searchInput) {
    searchInput.addEventListener('input', (e) => {
        const query = e.target.value.toLowerCase();
        const rawQuery = query.replace('#', '');
        
        if (query === '') {
            renderNotes();
            return;
        }
        
        // 1. Process folder groups
        const folderGroups = document.querySelectorAll('.folder-group');
        folderGroups.forEach(group => {
            const folderText = group.querySelector('.folder-header')?.textContent.toLowerCase() || '';
            const folderMatches = folderText.includes(query);
            
            const children = Array.from(group.querySelectorAll('.note-item'));
            let hasVisibleChild = false;
            children.forEach(item => {
                const title = item.querySelector('.note-title')?.textContent.toLowerCase() || '';
                const preview = item.querySelector('.note-preview')?.textContent.toLowerCase() || '';
                const tags = item.dataset.tags?.toLowerCase() || '';
                
                if (folderMatches || title.includes(query) || preview.includes(query) || tags.includes(rawQuery)) {
                    item.style.display = 'flex';
                    hasVisibleChild = true;
                } else {
                    item.style.display = 'none';
                }
            });
            
            if (hasVisibleChild || folderMatches) {
                group.style.display = 'block';
                group.querySelector('.folder-header')?.classList.remove('collapsed');
                group.querySelector('.folder-content')?.classList.remove('collapsed');
            } else {
                group.style.display = 'none';
            }
        });

        // 2. Process root notes
        const rootNotes = document.querySelectorAll('#notes-list > .note-item');
        rootNotes.forEach(item => {
            const title = item.querySelector('.note-title')?.textContent.toLowerCase() || '';
            const preview = item.querySelector('.note-preview')?.textContent.toLowerCase() || '';
            const tags = item.dataset.tags?.toLowerCase() || '';
            
            if (title.includes(query) || preview.includes(query) || tags.includes(rawQuery)) {
                item.style.display = 'flex';
            } else {
                item.style.display = 'none';
            }
        });
    });
}

// Sidebar Toggle Logic
const sidebarToggleBtn = document.getElementById('sidebar-toggle-btn');
const appContainer = document.getElementById('app');
if (sidebarToggleBtn && appContainer) {
    sidebarToggleBtn.onclick = () => {
        appContainer.classList.toggle('sidebar-collapsed');
    };
}

// Global Keyboard Shortcuts
document.addEventListener('keydown', (e) => {
    // Cmd (Mac) or Ctrl (Windows)
    const isCmd = e.metaKey || e.ctrlKey;
    
    if (isCmd && !e.shiftKey && e.key.toLowerCase() === 'n') {
        e.preventDefault();
        document.getElementById('new-note-btn')?.click();
    }
    else if (isCmd && e.shiftKey && e.key.toLowerCase() === 'n') {
        e.preventDefault();
        document.getElementById('new-folder-btn')?.click();
    }
    else if (isCmd && e.shiftKey && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        document.body.classList.toggle('focus-mode');
    }
    else if (isCmd && !e.shiftKey && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        document.getElementById('search-input')?.focus();
    }
    else if (isCmd && e.key.toLowerCase() === 'p') {
        e.preventDefault();
        document.getElementById('print-btn')?.click();
    }
});

// Help Modal Logic
const helpModal = document.getElementById('help-modal');
const closeHelpBtn = document.getElementById('close-help-btn');
const helpSearchInput = document.getElementById('help-search-input');
const helpSections = document.querySelectorAll('.help-section');

window.showHelpModal = function() {
    if (helpModal) {
        helpModal.style.display = 'flex';
        if (helpSearchInput) helpSearchInput.focus();
    }
};

if (closeHelpBtn && helpModal) {
    closeHelpBtn.onclick = () => {
        helpModal.style.display = 'none';
        if (helpSearchInput) helpSearchInput.value = '';
        if (helpSections) helpSections.forEach(s => s.style.display = 'block');
    };
}

if (helpSearchInput) {
    helpSearchInput.addEventListener('input', (e) => {
        const query = e.target.value.toLowerCase();
        helpSections.forEach(section => {
            const text = section.textContent.toLowerCase();
            if (text.includes(query)) {
                section.style.display = 'block';
            } else {
                section.style.display = 'none';
            }
        });
    });
}

// showAlert is defined at the top of the file
