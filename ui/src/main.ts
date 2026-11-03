import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';

const target = document.getElementById('grainatlas')!;
export default mount(App, { target });
