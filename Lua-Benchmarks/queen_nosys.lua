local N = tonumber(arg[1] or 8)
local function isplaceok (a, n, c)
  for i = 1, n - 1 do
    if (a[i] == c) or (a[i] - i == c - n) or (a[i] + i == c + n) then
      return false
    end
  end
  return true
end
local function addqueen (a, n)
  if n > N then
    local b = {}
    for i = 1, N do
      for j = 1, N do
        b[#b+1] = (a[i] == j and "X" or "-")
      end
    end
  else
    for c = 1, N do
      if isplaceok(a, n, c) then
        a[n] = c
        addqueen(a, n + 1)
      end
    end
  end
end
addqueen({}, 1)
