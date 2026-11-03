import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';

const target = document.getElementById('grainmap')!;
export default mount(App, { target });
