import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';
import { appStorage } from './lib/app-storage';

const target = document.getElementById('grainatlas')!;
const loading = setTimeout(() => {
  const text = document.createElement('p');
  text.className = 'faint small';
  text.textContent = 'Loading saved projects…';
  target.replaceChildren(text);
}, 150);
await appStorage.ready();
clearTimeout(loading);
target.replaceChildren();
export default mount(App, { target });
