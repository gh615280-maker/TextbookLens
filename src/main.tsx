import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

const root = document.getElementById('root');

if (root) {
  createRoot(root).render(<StrictMode>TextbookLens</StrictMode>);
}
