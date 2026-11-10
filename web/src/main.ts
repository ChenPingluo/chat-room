import './styles.css';
import { mount } from 'svelte';
import App from './App.svelte';

const root = document.getElementById('app');
if (root) mount(App, { target: root });

if ('serviceWorker' in navigator) {
  navigator.serviceWorker.register('/sw.js').catch(() => {});
}
