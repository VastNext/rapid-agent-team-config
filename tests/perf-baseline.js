// 本地前端性能基线：不读取真实配置，只测合成模型搜索与候选渲染。
const fs = require('fs');
const vm = require('vm');

const source = fs.readFileSync('src/frontend/app.js', 'utf8');
const sandbox = {
  console,
  setTimeout,
  clearTimeout,
  document: { getElementById: () => null, querySelectorAll: () => [] },
  window: { addEventListener: () => {} },
};
vm.createContext(sandbox);
vm.runInContext(`${source}\nthis.__fuzzyMatch = fuzzyMatch;`, sandbox);

const models = Array.from({ length: 10000 }, (_, index) =>
  `provider/model-${index}-gemini-flash`
);
const queries = ['gemini flash', 'model 9999', 'zhipu glm53', 'not-found'];
const results = queries.map(query => {
  const start = performance.now();
  let matches = 0;
  for (const model of models) {
    if (sandbox.__fuzzyMatch(model, query)) matches += 1;
  }
  return { query, matches, elapsed_ms: +(performance.now() - start).toFixed(3) };
});

console.log(JSON.stringify({ model_count: models.length, results }, null, 2));
