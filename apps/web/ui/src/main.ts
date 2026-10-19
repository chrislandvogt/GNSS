import { mount } from 'svelte'
import './app.css'
import App from './App.svelte'
import { installHarness } from './lib/harness'

if (new URLSearchParams(location.search).has('harness')) installHarness()

// Cache this version for offline use. Only in production builds, so
// development always loads fresh files.
if (import.meta.env.PROD && 'serviceWorker' in navigator) {
  navigator.serviceWorker.register('./sw.js').catch((err) => console.warn('offline cache unavailable:', err))
}

export default mount(App, { target: document.getElementById('app')! })
