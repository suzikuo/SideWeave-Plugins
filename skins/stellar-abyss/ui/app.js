(() => {
  const api = window.agileTilesSkin
  if (!api) return

  const byId = (id) => document.getElementById(id)
  const body = document.body
  const surface = body.dataset.surface
  let latestContext = {}

  function setStatus(message) {
    let node = byId('skin-status')
    if (!node) {
      node = document.createElement('div')
      node.id = 'skin-status'
      node.className = 'skin-toast'
      node.setAttribute('role', 'status')
      body.append(node)
    }
    node.textContent = message
    node.hidden = false
    window.clearTimeout(setStatus.timer)
    setStatus.timer = window.setTimeout(() => { node.hidden = true }, 2800)
  }

  async function call(action, payload = {}) {
    try {
      return await api.call(action, payload)
    } catch (error) {
      setStatus(String(error))
      return undefined
    }
  }

  function iconFor(plugin) {
    return typeof plugin.icon === 'string' && plugin.icon.trim() ? plugin.icon.trim() : '✦'
  }

  function renderPluginIcon(container, plugin) {
    const source = iconFor(plugin)
    if (source.startsWith('http://agile-plugin.localhost/')) {
      const mask = document.createElement('span')
      mask.className = 'plugin-icon-mask'
      mask.style.maskImage = `url("${source}")`
      mask.style.webkitMaskImage = `url("${source}")`
      const probe = document.createElement('img')
      probe.className = 'plugin-icon-probe'
      probe.src = source
      probe.alt = ''
      probe.setAttribute('aria-hidden', 'true')
      probe.addEventListener('error', () => {
        mask.remove()
        probe.remove()
        container.textContent = '✦'
      }, { once: true })
      container.append(mask, probe)
      return
    }
    const glyphs = { puzzle: '⬡', settings: '⌘', 'layout-dashboard': '▦', blocks: '▤', palette: '◉', monitor: '▣', cpu: '⌬', zap: 'ϟ' }
    container.textContent = glyphs[source] || '✦'
  }

  function descriptionFor(plugin) {
    return plugin.description || (plugin.enabled ? '模块在线 · 点击进入' : '模块待命 · 点击查看')
  }

  function makePluginButton(plugin, className, includeState = false) {
    const button = document.createElement('button')
    button.type = 'button'
    button.className = `${className}${plugin.enabled ? ' enabled' : ''}`
    button.setAttribute('aria-label', `${plugin.name}${includeState ? (plugin.enabled ? '，点击停用' : '，点击启用') : ''}`)
    button.title = `${plugin.name} · ${includeState ? (plugin.enabled ? '点击停用' : '点击启用') : '点击打开'}`
    const icon = document.createElement('span')
    icon.className = 'plugin-icon'
    icon.setAttribute('aria-hidden', 'true')
    renderPluginIcon(icon, plugin)
    const copy = document.createElement('span')
    copy.className = 'plugin-copy'
    const name = document.createElement('strong')
    name.textContent = plugin.name
    const description = document.createElement('small')
    description.textContent = descriptionFor(plugin)
    copy.append(name, description)
    button.append(icon, copy)
    if (includeState) {
      const state = document.createElement('span')
      state.className = 'module-state'
      button.append(state)
      button.addEventListener('click', async () => {
        await call('plugins.setEnabled', { pluginId: plugin.id, enabled: !plugin.enabled })
        await refreshContext()
      })
    } else if (className === 'plugin-card') {
      const arrow = document.createElement('span')
      arrow.className = 'plugin-arrow'
      arrow.textContent = '›'
      button.append(arrow)
      button.addEventListener('click', () => call('navigation.openPlugin', { pluginId: plugin.id }))
      button.addEventListener('mouseenter', () => {
        const rect = button.getBoundingClientRect()
        void call('sidebar.previewEnter', {
          pluginId: plugin.id,
          rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
        })
      })
      button.addEventListener('mouseleave', () => {
        void call('sidebar.previewLeave', { pluginId: plugin.id })
      })
    } else {
      button.addEventListener('click', () => call('navigation.openPlugin', { pluginId: plugin.id }))
    }
    return button
  }

  function renderTheme(settings) {
    const mode = settings?.appearance?.theme_mode || 'dark'
    document.querySelectorAll('[data-theme]').forEach((button) => {
      button.classList.toggle('active', button.dataset.theme === mode)
    })
    const label = byId('theme-label')
    if (label) label.textContent = mode === 'light' ? '星云模式' : mode === 'system' ? '跟随系统' : '深空模式'
  }

  function bindControls() {
    document.querySelectorAll('[data-action]').forEach((button) => {
      button.addEventListener('click', () => {
        const payload = button.dataset.page ? { page: button.dataset.page } : {}
        call(button.dataset.action, payload)
      })
    })
    document.querySelectorAll('[data-theme]').forEach((button) => {
      button.addEventListener('click', async () => {
        await call('settings.set', { category: 'appearance', key: 'theme_mode', value: button.dataset.theme })
        renderTheme(latestContext.settings)
      })
    })
  }

  function renderSidebar(context) {
    const list = byId('plugin-list')
    if (!list) return
    const appearance = context.settings?.appearance || {}
    const position = ['left', 'right', 'top'].includes(appearance.sidebar_position)
      ? appearance.sidebar_position
      : 'right'
    const railSize = Math.max(56, Math.min(96, Number(appearance.collapsed_width) || 64))
    body.dataset.position = position
    body.dataset.expanded = context.expanded === false ? 'false' : 'true'
    body.style.setProperty('--rail-size', `${railSize}px`)
    body.style.setProperty('--ring-size', `${Math.round(railSize * 0.56)}px`)

    const plugins = Array.isArray(context.plugins) ? context.plugins.filter((plugin) => plugin.enabled) : []
    list.replaceChildren()
    plugins.forEach((plugin, index) => {
      const button = makePluginButton(plugin, 'plugin-card')
      button.style.setProperty('--item-delay', `${Math.min(index, 8) * 38}ms`)
      list.append(button)
    })

    const settings = document.createElement('button')
    settings.type = 'button'
    settings.className = 'plugin-card system-card'
    settings.setAttribute('aria-label', '设置；右键打开控制中心')
    settings.title = '设置 · 右键打开控制中心'
    const icon = document.createElement('span')
    icon.className = 'plugin-icon system-icon'
    icon.setAttribute('aria-hidden', 'true')
    icon.textContent = '⌘'
    settings.append(icon)
    settings.addEventListener('click', () => call('navigation.openSettings'))
    settings.addEventListener('contextmenu', (event) => {
      event.preventDefault()
      void call('navigation.openControlCenter')
    })
    settings.style.setProperty('--item-delay', `${Math.min(plugins.length, 8) * 38}ms`)
    list.append(settings)
  }

  function renderDetail(context) {
    const page = context.page || 'home'
    const title = byId('page-title')
    const subtitle = byId('page-subtitle')
    const eyebrow = byId('page-eyebrow')
    const isSettings = page === 'settings'
    if (title) title.textContent = isSettings ? '航行设置' : '欢迎回到星舰'
    if (subtitle) subtitle.textContent = isSettings
      ? '调整星舰显示模式，打造适合你的深空工作环境。'
      : '你的数字宇宙已准备就绪。选择一个模块，继续探索。'
    if (eyebrow) eyebrow.textContent = isSettings ? '系统偏好 · 02' : '航行状态 · 01'
    renderTheme(context.settings)
    const panel = byId('appearance-panel')
    if (panel) panel.hidden = !isSettings
    const grid = byId('detail-plugin-grid')
    if (!grid) return
    grid.replaceChildren()
    const plugins = Array.isArray(context.plugins) ? context.plugins.filter((plugin) => plugin.enabled) : []
    for (const plugin of plugins.slice(0, 6)) {
      const button = makePluginButton(plugin, 'detail-plugin-card')
      grid.append(button)
    }
    if (!plugins.length) {
      const empty = document.createElement('div')
      empty.className = 'empty-state'
      empty.textContent = '暂无在线模块。打开控制中心以启用或安装插件。'
      grid.append(empty)
    }
  }

  function renderControl(context) {
    const plugins = Array.isArray(context.plugins) ? context.plugins : []
    const enabled = plugins.filter((plugin) => plugin.enabled).length
    const total = byId('enabled-count')
    const count = byId('module-count')
    if (total) total.textContent = `${enabled} / ${plugins.length}`
    if (count) count.textContent = String(plugins.length).padStart(2, '0')
    renderTheme(context.settings)
    const grid = byId('control-plugin-grid')
    if (!grid) return
    grid.replaceChildren()
    if (!plugins.length) {
      const empty = document.createElement('div')
      empty.className = 'empty-state'
      empty.textContent = '尚未安装星舰模块。你可以从设置页打开插件市场。'
      grid.append(empty)
      return
    }
    for (const plugin of plugins) grid.append(makePluginButton(plugin, 'module-card', true))
  }

  function renderPreview(context) {
    const preview = context.preview || {}
    const title = byId('preview-title')
    const subtitle = byId('preview-subtitle')
    const status = byId('preview-status')
    if (title) title.textContent = preview.title || '等待模块信号'
    if (subtitle) subtitle.textContent = preview.subtitle || '将鼠标停留在星舰模块上查看实时数据。'
    if (status) {
      status.textContent = preview.statusText || ({ online: '信号已连接', busy: '模块处理中', offline: '模块离线', error: '信号异常' }[preview.status] || '信号已连接')
      status.classList.toggle('preview-status-error', preview.status === 'error' || preview.status === 'offline')
      status.classList.toggle('preview-status-busy', preview.status === 'busy')
    }
    const metrics = byId('preview-metrics')
    if (metrics) {
      metrics.replaceChildren()
      for (const item of Array.isArray(preview.metrics) ? preview.metrics.slice(0, 4) : []) {
        const card = document.createElement('div')
        card.className = 'metric-card'
        const label = document.createElement('small')
        label.textContent = item.label || ''
        const value = document.createElement('strong')
        value.textContent = item.value ?? '—'
        card.append(label, value)
        metrics.append(card)
      }
    }
    const progress = byId('preview-progress')
    const progressInfo = preview.progress
    if (progress) {
      progress.hidden = !progressInfo
      if (progressInfo) {
        const value = Math.max(0, Math.min(100, Number(progressInfo.value) || 0))
        byId('progress-label').textContent = progressInfo.label || '任务进度'
        byId('progress-value').textContent = `${Math.round(value)}%`
        byId('progress-fill').style.width = `${value}%`
      }
    }
    const items = byId('preview-items')
    if (items) {
      items.replaceChildren()
      for (const item of Array.isArray(preview.items) ? preview.items.slice(0, 4) : []) {
        const row = document.createElement(item.method ? 'button' : 'div')
        row.className = 'preview-item'
        if (item.method) {
          row.type = 'button'
          row.classList.add('interactive')
          row.addEventListener('click', async () => {
            if (preview.pluginId) await call('plugins.call', { pluginId: preview.pluginId, method: item.method, payload: item.payload || {} })
            await call('preview.refresh')
          })
        }
        const title = document.createElement('span')
        title.className = 'preview-item-title'
        title.textContent = typeof item === 'string' ? item : (item.title || item.label || item.value || '')
        row.append(title)
        row.title = item.tooltip || title.textContent
        if (Array.isArray(item.tags) && item.tags.length) {
          const tags = document.createElement('span')
          tags.className = 'preview-item-tags'
          for (const tag of item.tags.slice(0, 2)) {
            const label = document.createElement('span')
            label.className = 'preview-group-tag'
            label.textContent = tag.label
            label.title = tag.label
            if (/^#[0-9a-f]{6}$/i.test(tag.color || '')) label.style.setProperty('--group-color', tag.color)
            tags.append(label)
          }
          if (item.tags.length > 2) {
            const more = document.createElement('span')
            more.className = 'preview-tag-more'
            more.textContent = `+${item.tags.length - 2}`
            more.title = item.tags.slice(2).map((tag) => tag.label).join('、')
            tags.append(more)
          }
          row.append(tags)
        }
        items.append(row)
      }
    }
    const actions = byId('preview-actions')
    if (actions) {
      actions.replaceChildren()
      const pluginId = preview.pluginId
      const actionList = Array.isArray(preview.actions) ? preview.actions.slice(0, 2) : []
      for (const action of actionList) {
        const button = document.createElement('button')
        button.type = 'button'
        button.className = `preview-action${action.primary ? ' primary' : ''}`
        button.textContent = action.label || '执行'
        button.addEventListener('click', async () => {
          if (action.method && pluginId) await call('plugins.call', { pluginId, method: action.method, payload: action.payload || {} })
          await call('preview.refresh')
        })
        actions.append(button)
      }
      if (pluginId) {
        const open = document.createElement('button')
        open.type = 'button'
        open.className = `preview-action${actionList.length ? '' : ' primary'}`
        open.textContent = '打开模块 ↗'
        open.addEventListener('click', () => call('navigation.openPlugin', { pluginId }))
        actions.append(open)
      }
    }
  }

  function render(context) {
    latestContext = context || {}
    body.dataset.theme = latestContext.theme?.mode || (latestContext.settings?.appearance?.theme_mode === 'light' ? 'light' : 'dark')
    if (surface === 'sidebar') renderSidebar(latestContext)
    if (surface === 'detail') renderDetail(latestContext)
    if (surface === 'control') renderControl(latestContext)
    if (surface === 'preview') renderPreview(latestContext)
  }

  async function refreshContext() {
    const state = await call('state.get')
    if (!state) return
    latestContext = { ...latestContext, ...state }
    render(latestContext)
  }

  bindControls()
  api.onContext(render)
  api.onEvent('settingsChanged', (settings) => render({ ...latestContext, settings }))
  api.onEvent('themeChanged', (theme) => render({ ...latestContext, theme }))
  api.onEvent('sidebarExpanded', (expanded) => {
    latestContext.expanded = expanded
    if (surface === 'sidebar') {
      body.dataset.expanded = expanded ? 'true' : 'false'
    }
  })
  api.onEvent('mainSurface', (page) => { latestContext.page = page; render(latestContext) })
})()
