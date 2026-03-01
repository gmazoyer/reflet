FROM node:24-alpine AS builder

WORKDIR /build

# Install dependencies first for better layer caching
COPY package.json package-lock.json ./
RUN npm ci

# Copy source and build
COPY . .
RUN npm run build

FROM nginx:alpine

# Remove default nginx config
RUN rm /etc/nginx/conf.d/default.conf

# Copy built SPA assets
COPY --from=builder /build/dist /usr/share/nginx/html

# Copy nginx config template
COPY nginx.conf /etc/nginx/templates/default.conf.template

# BACKEND_URL is resolved by envsubst at container start.
# nginx:alpine automatically processes /etc/nginx/templates/*.template files.
# NGINX_ENVSUBST_FILTER restricts substitution to BACKEND_URL only,
# so nginx variables like $uri, $host, $proxy_add_x_forwarded_for are left intact.
ENV BACKEND_URL=http://backend:8080
ENV NGINX_ENVSUBST_FILTER=BACKEND_URL

EXPOSE 80
