FROM python:3.11-slim

WORKDIR /app

COPY requirements-server.txt .
RUN pip install --no-cache-dir -r requirements-server.txt

COPY server.py .

EXPOSE 8765

CMD ["python", "-u", "server.py"]
